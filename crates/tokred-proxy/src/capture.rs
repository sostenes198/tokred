//! Content capture for the dashboard (only with `[proxy] log_bodies = true`): what a request
//! sent that the previous request of the same conversation had not, and what the response
//! said. The whole conversation travels with every request, so the request side keeps a hash
//! per message and reports only the messages after the ones it has already seen.

use std::collections::{HashMap, VecDeque};
use std::io::Write;

use serde_json::Value;
use tokred_core::dashboard::{Clipped, MAX_TEXT_CHARS, ResponseView, SentMessage, ToolUseView};

/// Conversations remembered by the request side.
const TRACKED_CONVERSATIONS: usize = 256;
/// The most messages one request reports (a resumed conversation sends its whole history once).
const MAX_SENT_MESSAGES: usize = 200;
/// Longest summary of a tool input, in characters.
const SUMMARY_CHARS: usize = 200;
/// Tool input kept while it streams in, in bytes (the rest is dropped).
const MAX_TOOL_JSON: usize = 64 * 1024;
/// Input fields that say best what a tool call does, in order of preference.
const SUMMARY_KEYS: [&str; 9] = [
    "command",
    "file_path",
    "path",
    "pattern",
    "query",
    "url",
    "description",
    "prompt",
    "text",
];

fn clip(text: &str, limit: usize) -> String {
    let mut clipped = Clipped::new(limit);
    clipped.push_str(text);
    clipped.finish()
}

/// A short, human-readable summary of a tool call's input.
pub(crate) fn summarize_input(input: &Value) -> String {
    let named = input.as_object().and_then(|object| {
        SUMMARY_KEYS
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_str))
    });
    match (named, input) {
        (Some(text), _) => clip(text, SUMMARY_CHARS),
        (None, Value::String(text)) => clip(text, SUMMARY_CHARS),
        (None, other) => clip(&other.to_string(), SUMMARY_CHARS),
    }
}

/// The text of a `tool_result` content: a string, or the text blocks of an array.
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(|block| match block.get("type").and_then(Value::as_str) {
                Some("text") => block
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                other => format!("[{}]", other.unwrap_or("block")),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// One content block as a line of text.
fn block_text(block: &Value) -> String {
    let text_of = |key: &str| {
        block
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    match block.get("type").and_then(Value::as_str) {
        Some("text") => text_of("text"),
        Some("thinking") => format!("[thinking] {}", text_of("thinking")),
        Some("tool_use") => format!(
            "[tool_use {}] {}",
            text_of("name"),
            summarize_input(block.get("input").unwrap_or(&Value::Null))
        ),
        Some("tool_result") => format!("[tool_result] {}", result_text(block.get("content"))),
        Some(other) => format!("[{other}]"),
        None => "[block]".to_owned(),
    }
}

/// A request message as the dashboard shows it: the role and its blocks as text, truncated.
pub(crate) fn message_view(message: &Value) -> SentMessage {
    let role = match message.get("role").and_then(Value::as_str) {
        Some("assistant") => "assistant",
        _ => "user",
    };
    let mut text = Clipped::new(MAX_TEXT_CHARS);
    match message.get("content") {
        Some(Value::String(content)) => text.push_str(content),
        Some(Value::Array(blocks)) => {
            for (index, block) in blocks.iter().enumerate() {
                if index > 0 {
                    text.push_str("\n\n");
                }
                text.push_str(&block_text(block));
            }
        }
        _ => {}
    }
    SentMessage {
        role: role.to_owned(),
        text: text.finish(),
    }
}

/// The system prompt of a request (a string, or the text blocks of an array).
fn system_text(system: &Value) -> String {
    match system {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

/// Adapts a hasher to the writer `serde_json` wants, so values are hashed without building
/// their text first.
struct HashWriter<'a>(&'a mut blake3::Hasher);

impl Write for HashWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A 64-bit fingerprint of a JSON value.
fn fingerprint(value: &Value) -> u64 {
    let mut hasher = blake3::Hasher::new();
    // Writing to a hasher cannot fail, and a value always serializes.
    let _written = serde_json::to_writer(HashWriter(&mut hasher), value);
    let hash = hasher.finalize();
    let mut head = [0_u8; 8];
    head.copy_from_slice(&hash.as_bytes()[..8]);
    u64::from_le_bytes(head)
}

/// What was seen of one conversation (a session, or one subagent of it).
#[derive(Debug, Default)]
struct Seen {
    system: u64,
    messages: Vec<u64>,
}

/// Finds the messages of each request that the previous request of the same conversation did
/// not carry.
#[derive(Debug, Default)]
pub(crate) struct SentTracker {
    seen: HashMap<(String, String), Seen>,
    order: VecDeque<(String, String)>,
}

impl SentTracker {
    /// The messages of `body` that are new for the conversation `(session, agent)`: the whole
    /// conversation the first time (the system prompt first), the tail afterwards, and what
    /// differs from the first changed message on when the history was rewritten (compaction).
    pub(crate) fn new_messages(
        &mut self,
        session: &str,
        agent: Option<&str>,
        body: &Value,
    ) -> Vec<SentMessage> {
        let key = (session.to_owned(), agent.unwrap_or_default().to_owned());
        let messages: &[Value] = body
            .get("messages")
            .and_then(Value::as_array)
            .map_or(&[], Vec::as_slice);
        let system = body.get("system").unwrap_or(&Value::Null);
        let system_hash = fingerprint(system);

        let seen = self.entry(key);
        let first_time = seen.messages.is_empty() && seen.system == 0;
        let mut out = Vec::new();
        if first_time || seen.system != system_hash {
            let text = system_text(system);
            if !text.is_empty() {
                out.push(SentMessage {
                    role: "system".to_owned(),
                    text: clip(&text, MAX_TEXT_CHARS),
                });
            }
            seen.system = system_hash;
        }

        let known = common_prefix(seen, messages);
        let fresh = messages.get(known..).unwrap_or_default();
        seen.messages.truncate(known);
        seen.messages.extend(fresh.iter().map(fingerprint));
        let skip = fresh.len().saturating_sub(MAX_SENT_MESSAGES);
        out.extend(fresh.iter().skip(skip).map(message_view));
        out
    }

    fn entry(&mut self, key: (String, String)) -> &mut Seen {
        if !self.seen.contains_key(&key) {
            if self.order.len() >= TRACKED_CONVERSATIONS
                && let Some(oldest) = self.order.pop_front()
            {
                self.seen.remove(&oldest);
            }
            self.order.push_back(key.clone());
        }
        self.seen.entry(key).or_default()
    }
}

/// How many leading messages are the ones already seen. The usual case (the same history
/// plus new messages) is decided from the first and the last message seen; a request that
/// disagrees is compared message by message.
fn common_prefix(seen: &Seen, messages: &[Value]) -> usize {
    let known = seen.messages.len();
    let hash_at = |index: usize| messages.get(index).map(fingerprint);
    let appended = known > 0
        && known <= messages.len()
        && hash_at(0) == seen.messages.first().copied()
        && hash_at(known - 1) == seen.messages.last().copied();
    if appended {
        return known;
    }
    seen.messages
        .iter()
        .zip(messages)
        .take_while(|(hash, message)| **hash == fingerprint(message))
        .count()
}

/// A tool call being assembled from a stream: its name and the input JSON as it arrives.
#[derive(Debug)]
struct ToolBlock {
    name: String,
    json: String,
}

/// Reassembles a response from its stream events (or from its JSON body).
#[derive(Debug)]
pub(crate) struct ResponseCapture {
    text: Clipped,
    thinking: Clipped,
    tool_uses: Vec<ToolUseView>,
    tools: HashMap<u64, ToolBlock>,
    pending: String,
}

impl ResponseCapture {
    pub(crate) fn new() -> Self {
        Self {
            text: Clipped::new(MAX_TEXT_CHARS),
            thinking: Clipped::new(MAX_TEXT_CHARS),
            tool_uses: Vec::new(),
            tools: HashMap::new(),
            pending: String::new(),
        }
    }

    /// The text received since the last call, for the live stream.
    pub(crate) fn take_delta(&mut self) -> Option<String> {
        (!self.pending.is_empty()).then(|| std::mem::take(&mut self.pending))
    }

    fn add_text(&mut self, text: &str) {
        self.text.push_str(text);
        self.pending.push_str(text);
    }

    /// A `content_block_start`, `content_block_delta` or `content_block_stop` event.
    pub(crate) fn event(&mut self, event: &Value) {
        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
        match event.get("type").and_then(Value::as_str) {
            Some("content_block_start") => {
                let block = event.get("content_block").unwrap_or(&Value::Null);
                self.start(index, block);
            }
            Some("content_block_delta") => {
                self.delta(index, event.get("delta").unwrap_or(&Value::Null));
            }
            Some("content_block_stop") => {
                if let Some(tool) = self.tools.remove(&index) {
                    self.finish_tool(tool);
                }
            }
            _ => {}
        }
    }

    fn start(&mut self, index: u64, block: &Value) {
        let field = |name: &str| block.get(name).and_then(Value::as_str).unwrap_or("");
        let kind = field("type");
        if kind == "text" {
            self.add_text(field("text"));
        } else if kind.ends_with("tool_use") {
            let tool = ToolBlock {
                name: field("name").to_owned(),
                json: String::new(),
            };
            self.tools.insert(index, tool);
        }
    }

    fn delta(&mut self, index: u64, delta: &Value) {
        let field = |name: &str| delta.get(name).and_then(Value::as_str).unwrap_or("");
        match delta.get("type").and_then(Value::as_str) {
            Some("text_delta") => self.add_text(field("text")),
            Some("thinking_delta") => self.thinking.push_str(field("thinking")),
            Some("input_json_delta") => {
                if let Some(tool) = self.tools.get_mut(&index)
                    && tool.json.len() < MAX_TOOL_JSON
                {
                    tool.json.push_str(field("partial_json"));
                }
            }
            _ => {}
        }
    }

    fn finish_tool(&mut self, tool: ToolBlock) {
        let input_summary = serde_json::from_str::<Value>(&tool.json).map_or_else(
            |_| clip(&tool.json, SUMMARY_CHARS),
            |input| summarize_input(&input),
        );
        self.tool_uses.push(ToolUseView {
            name: tool.name,
            input_summary,
        });
    }

    /// A whole (non-streaming) message body.
    pub(crate) fn message(&mut self, message: &Value) {
        let blocks = message
            .get("content")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        for block in blocks {
            let field = |name: &str| block.get(name).and_then(Value::as_str).unwrap_or("");
            match field("type") {
                "text" => self.add_text(field("text")),
                "thinking" => self.thinking.push_str(field("thinking")),
                kind if kind.ends_with("tool_use") => self.tool_uses.push(ToolUseView {
                    name: field("name").to_owned(),
                    input_summary: summarize_input(block.get("input").unwrap_or(&Value::Null)),
                }),
                _ => {}
            }
        }
    }

    /// The reassembled response. Tool calls whose stream was cut are included with what
    /// arrived.
    pub(crate) fn finish(mut self) -> ResponseView {
        let mut open: Vec<(u64, ToolBlock)> = self.tools.drain().collect();
        open.sort_by_key(|(index, _)| *index);
        for (_, tool) in open {
            self.finish_tool(tool);
        }
        let thinking = (!self.thinking.is_empty()).then(|| self.thinking.finish());
        ResponseView {
            text: self.text.finish(),
            thinking,
            tool_uses: self.tool_uses,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn texts(messages: &[SentMessage]) -> Vec<String> {
        messages
            .iter()
            .map(|m| format!("{}:{}", m.role, m.text))
            .collect()
    }

    fn body(system: &str, messages: &[Value]) -> Value {
        json!({"system": system, "messages": messages})
    }

    fn user(text: &str) -> Value {
        json!({"role": "user", "content": text})
    }

    fn assistant(text: &str) -> Value {
        json!({"role": "assistant", "content": [{"type": "text", "text": text}]})
    }

    #[test]
    fn tool_inputs_are_summarized_by_their_most_telling_field() {
        assert_eq!(
            summarize_input(&json!({"file_path": "/a/b.rs", "limit": 5})),
            "/a/b.rs"
        );
        assert_eq!(
            summarize_input(&json!({"pattern": "fn main", "command": "ls"})),
            "ls",
            "command wins over pattern"
        );
        assert_eq!(summarize_input(&json!("plain")), "plain");
        assert_eq!(summarize_input(&json!({"n": 1})), "{\"n\":1}");
        let long = summarize_input(&json!({"command": "x".repeat(300)}));
        assert!(
            long.starts_with(&"x".repeat(SUMMARY_CHARS)) && long.ends_with("[truncated 100 chars]")
        );
    }

    #[test]
    fn messages_become_role_and_text_with_their_blocks_spelled_out() {
        let message = json!({"role": "user", "content": [
            {"type": "text", "text": "look"},
            {"type": "tool_result", "tool_use_id": "t1", "content": "fn main() {}"},
            {"type": "tool_result", "tool_use_id": "t2", "content": [
                {"type": "text", "text": "line"}, {"type": "image"}, {}]},
            {"type": "tool_result", "tool_use_id": "t3"},
            {"type": "image"},
            {"text": "no type"}
        ]});
        let view = message_view(&message);
        assert_eq!(view.role, "user");
        assert_eq!(
            view.text,
            "look\n\n[tool_result] fn main() {}\n\n[tool_result] line\n[image]\n[block]\n\n[tool_result] \n\n[image]\n\n[block]"
        );
        let assistant = message_view(&json!({"role": "assistant", "content": [
            {"type": "thinking", "thinking": "hmm"},
            {"type": "tool_use", "name": "Read", "input": {"file_path": "/x"}}]}));
        assert_eq!(assistant.role, "assistant");
        assert_eq!(assistant.text, "[thinking] hmm\n\n[tool_use Read] /x");
        assert_eq!(message_view(&user("plain")).text, "plain");
        assert_eq!(
            message_view(&json!({"role": "system", "content": 3})).role,
            "user"
        );
        assert_eq!(message_view(&json!({})).text, "");
        let huge = message_view(&user(&"é".repeat(MAX_TEXT_CHARS + 7)));
        assert!(huge.text.ends_with("…[truncated 7 chars]"));
    }

    #[test]
    fn the_first_request_reports_everything_and_later_ones_only_the_tail() {
        let mut tracker = SentTracker::default();
        let first = tracker.new_messages("s", None, &body("be brief", &[user("hi")]));
        assert_eq!(texts(&first), ["system:be brief", "user:hi"]);

        let second = tracker.new_messages(
            "s",
            None,
            &body("be brief", &[user("hi"), assistant("hello"), user("more")]),
        );
        assert_eq!(texts(&second), ["assistant:hello", "user:more"]);

        let same = tracker.new_messages(
            "s",
            None,
            &body("be brief", &[user("hi"), assistant("hello"), user("more")]),
        );
        assert!(same.is_empty(), "a repeated request has nothing new");

        let changed_system = tracker.new_messages(
            "s",
            None,
            &body(
                "be verbose",
                &[user("hi"), assistant("hello"), user("more")],
            ),
        );
        assert_eq!(texts(&changed_system), ["system:be verbose"]);
    }

    #[test]
    fn subagents_and_sessions_are_tracked_separately() {
        let mut tracker = SentTracker::default();
        let main = body("", &[user("main task")]);
        let sub = body("", &[user("sub task")]);
        assert_eq!(
            texts(&tracker.new_messages("s", None, &main)),
            ["user:main task"]
        );
        assert_eq!(
            texts(&tracker.new_messages("s", Some("a1"), &sub)),
            ["user:sub task"]
        );
        assert_eq!(
            texts(&tracker.new_messages("t", None, &main)),
            ["user:main task"]
        );
        assert!(tracker.new_messages("s", None, &main).is_empty());
    }

    #[test]
    fn a_rewritten_history_reports_what_differs() {
        let mut tracker = SentTracker::default();
        let history = [user("a"), assistant("b"), user("c"), assistant("d")];
        tracker.new_messages("s", None, &body("", &history));

        // Compaction: the history shrinks to a summary and the latest messages.
        let compacted = [user("summary of a b c"), assistant("d"), user("e")];
        let sent = tracker.new_messages("s", None, &body("", &compacted));
        assert_eq!(
            texts(&sent),
            ["user:summary of a b c", "assistant:d", "user:e"]
        );

        // A rewind: the conversation goes back to an earlier message and continues differently.
        let before = [user("a"), assistant("b"), user("c")];
        let mut tracker = SentTracker::default();
        tracker.new_messages("s", None, &body("", &before));
        let edited = [user("a"), assistant("B"), user("C")];
        let sent = tracker.new_messages("s", None, &body("", &edited));
        assert_eq!(texts(&sent), ["assistant:B", "user:C"]);

        // Fewer messages than before and a different first one: everything is new.
        let shorter = tracker.new_messages("s", None, &body("", &[user("z")]));
        assert_eq!(texts(&shorter), ["user:z"]);
        // A body without messages reports nothing.
        assert!(tracker.new_messages("s", None, &json!({})).is_empty());
    }

    #[test]
    fn a_long_history_reports_only_its_newest_messages() {
        let mut tracker = SentTracker::default();
        let many: Vec<Value> = (0..MAX_SENT_MESSAGES + 5)
            .map(|n| user(&format!("m{n}")))
            .collect();
        let sent = tracker.new_messages("s", None, &body("", &many));
        assert_eq!(sent.len(), MAX_SENT_MESSAGES);
        assert_eq!(sent[0].text, "m5");
    }

    #[test]
    fn the_tracker_forgets_the_oldest_conversation_when_full() {
        let mut tracker = SentTracker::default();
        for n in 0..=TRACKED_CONVERSATIONS {
            tracker.new_messages(&format!("s{n}"), None, &body("", &[user("x")]));
        }
        assert_eq!(tracker.seen.len(), TRACKED_CONVERSATIONS);
        // The first one was forgotten: its message is new again.
        assert_eq!(
            tracker
                .new_messages("s0", None, &body("", &[user("x")]))
                .len(),
            1
        );
        // A recent one is still remembered.
        let last = format!("s{TRACKED_CONVERSATIONS}");
        assert!(
            tracker
                .new_messages(&last, None, &body("", &[user("x")]))
                .is_empty()
        );
    }

    fn stream(events: &[Value]) -> (ResponseView, Vec<String>) {
        let mut capture = ResponseCapture::new();
        let mut deltas = Vec::new();
        for event in events {
            capture.event(event);
            deltas.extend(capture.take_delta());
        }
        (capture.finish(), deltas)
    }

    #[test]
    fn a_stream_is_reassembled_with_its_deltas_and_tool_inputs() {
        let (response, deltas) = stream(&[
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "let me "}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "think"}}),
            json!({"type": "content_block_stop", "index": 0}),
            json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": "He"}}),
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "llo"}}),
            json!({"type": "content_block_stop", "index": 1}),
            json!({"type": "content_block_start", "index": 2, "content_block": {"type": "tool_use", "id": "t", "name": "Read", "input": {}}}),
            json!({"type": "content_block_delta", "index": 2, "delta": {"type": "input_json_delta", "partial_json": "{\"file_pa"}}),
            json!({"type": "content_block_delta", "index": 2, "delta": {"type": "input_json_delta", "partial_json": "th\":\"/a.rs\"}"}}),
            json!({"type": "content_block_stop", "index": 2}),
            json!({"type": "content_block_start", "index": 3, "content_block": {"type": "web_search_tool_result"}}),
            json!({"type": "content_block_delta", "index": 3, "delta": {"type": "citations_delta"}}),
            json!({"type": "content_block_delta", "index": 9, "delta": {"type": "input_json_delta", "partial_json": "{}"}}),
            json!({"type": "content_block_stop", "index": 3}),
            json!({"type": "ping"}),
        ]);
        assert_eq!(deltas, ["He", "llo"], "the text, not the thinking, streams");
        assert_eq!(response.text, "Hello");
        assert_eq!(response.thinking.as_deref(), Some("let me think"));
        assert_eq!(
            response.tool_uses,
            [ToolUseView {
                name: "Read".to_owned(),
                input_summary: "/a.rs".to_owned()
            }]
        );
    }

    #[test]
    fn a_cut_stream_keeps_the_tools_it_had_started_and_odd_input_is_summarized_raw() {
        let (response, _) = stream(&[
            json!({"type": "content_block_start", "index": 1, "content_block": {"type": "server_tool_use", "name": "web_search"}}),
            json!({"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"query\": \"ru"}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use"}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": "{\"command\":\"ls\"}"}}),
        ]);
        let names: Vec<(&str, &str)> = response
            .tool_uses
            .iter()
            .map(|t| (t.name.as_str(), t.input_summary.as_str()))
            .collect();
        assert_eq!(names, [("", "ls"), ("web_search", "{\"query\": \"ru")]);
        assert_eq!(response.text, "");
        assert_eq!(response.thinking, None);
    }

    #[test]
    fn tool_input_that_keeps_coming_is_capped() {
        let mut capture = ResponseCapture::new();
        capture.event(&json!({"type": "content_block_start", "index": 0, "content_block": {"type": "tool_use", "name": "Write"}}));
        let chunk = "x".repeat(MAX_TOOL_JSON);
        for _ in 0..3 {
            capture.event(&json!({"type": "content_block_delta", "index": 0, "delta": {"type": "input_json_delta", "partial_json": chunk}}));
        }
        let view = capture.finish();
        assert!(
            view.tool_uses[0]
                .input_summary
                .ends_with("[truncated 800 chars]")
                || view.tool_uses[0].input_summary.contains("truncated")
        );
    }

    #[test]
    fn a_json_body_is_read_like_a_stream() {
        let mut capture = ResponseCapture::new();
        capture.message(&json!({"content": [
            {"type": "thinking", "thinking": "hm"},
            {"type": "text", "text": "answer"},
            {"type": "tool_use", "name": "Grep", "input": {"pattern": "todo"}},
            {"type": "image"}
        ]}));
        assert_eq!(capture.take_delta().as_deref(), Some("answer"));
        assert_eq!(capture.take_delta(), None);
        let view = capture.finish();
        assert_eq!(
            (view.text.as_str(), view.thinking.as_deref()),
            ("answer", Some("hm"))
        );
        assert_eq!(view.tool_uses[0].input_summary, "todo");
        let mut empty = ResponseCapture::new();
        empty.message(&json!({"id": "m"}));
        assert_eq!(empty.finish(), ResponseView::default());
    }

    #[test]
    fn fingerprints_follow_the_content() {
        assert_eq!(fingerprint(&json!({"a": 1})), fingerprint(&json!({"a": 1})));
        assert_ne!(fingerprint(&json!({"a": 1})), fingerprint(&json!({"a": 2})));
        let mut hasher = blake3::Hasher::new();
        let mut writer = HashWriter(&mut hasher);
        writer.flush().unwrap();
        assert_eq!(writer.write(b"abc").unwrap(), 3);
    }
}
