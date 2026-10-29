//! Domain of the local proxy: what a request is, who sent it, what the proxy may do to it, and
//! what it records. Pure: no I/O, no async. The `tokred-proxy` crate does the networking and
//! calls into this.
//!
//! The rule that the **main loop's model is never changed** is enforced by types: rewriting the
//! model of a request requires a [`RewriteTicket`], and a ticket can only be obtained (through
//! [`rewrite_ticket`]) for a subagent or an auxiliary request.

use serde::Serialize;
use serde_json::Value;

use crate::compression::{CompressedBlock, ShadowRecord, StoredResult};
use crate::dashboard::CapturedTurn;
use crate::error::StoreError;
use crate::pricing::Usage;
use crate::prompts::{PromptSummary, SuggestionRecord};
use crate::routing::RoutingDecision;
use crate::types::Timestamp;

/// Metadata Claude Code sends in `x-claude-code-*` headers. In Claude Code 2.1.312 only
/// `session-id` (always) and `agent-id` (subagents) were observed; the others are documented
/// and used when present.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RequestMeta {
    /// `x-claude-code-session-id`.
    pub session_id: Option<String>,
    /// `x-claude-code-agent-id` (present only on a subagent's requests).
    pub agent_id: Option<String>,
    /// `x-claude-code-parent-agent-id`.
    pub parent_agent_id: Option<String>,
    /// `x-claude-code-agent-type` (`Explore`, `Plan`, `general-purpose`, `custom`, ...).
    pub agent_type: Option<String>,
    /// `x-claude-code-request-class` (`main`, `subagent`, `workflow`, `compaction`, `auxiliary`).
    pub request_class: Option<String>,
    /// `x-claude-code-prompt-id`.
    pub prompt_id: Option<String>,
    /// `x-claude-code-compaction` (`auto`, `manual`, `reactive`).
    pub compaction: Option<String>,
    /// `x-claude-code-context-compacted`.
    pub context_compacted: Option<String>,
    /// The values of `anthropic-beta` (capabilities requested with the body, e.g. 1M context).
    pub betas: Vec<String>,
}

impl RequestMeta {
    /// Reads the metadata from request headers (names compared case-insensitively; empty
    /// values ignored).
    #[must_use]
    pub fn from_headers<'a>(headers: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut meta = Self::default();
        for (name, value) in headers {
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            let name = name.to_ascii_lowercase();
            if name == "anthropic-beta" {
                meta.betas.extend(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|beta| !beta.is_empty())
                        .map(str::to_owned),
                );
                continue;
            }
            let slot = match name.as_str() {
                "x-claude-code-session-id" => &mut meta.session_id,
                "x-claude-code-agent-id" => &mut meta.agent_id,
                "x-claude-code-parent-agent-id" => &mut meta.parent_agent_id,
                "x-claude-code-agent-type" => &mut meta.agent_type,
                "x-claude-code-request-class" => &mut meta.request_class,
                "x-claude-code-prompt-id" => &mut meta.prompt_id,
                "x-claude-code-compaction" => &mut meta.compaction,
                "x-claude-code-context-compacted" => &mut meta.context_compacted,
                _ => continue,
            };
            *slot = Some(value.to_owned());
        }
        meta
    }
}

/// The structural facts of a `/v1/messages` body the proxy decides on. Never message contents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RequestShape {
    /// `model`.
    pub model: Option<String>,
    /// Names of the tools offered, in order.
    pub tool_names: Vec<String>,
    /// A `thinking` field is present.
    pub has_thinking: bool,
    /// Number of entries in `messages`.
    pub message_count: usize,
    /// `stream`.
    pub stream: bool,
    /// The last message is a user turn typed by the person (text and no `tool_result`), i.e. the
    /// request starts a new prompt rather than continuing a tool loop.
    pub starts_prompt: bool,
}

impl RequestShape {
    /// Reads the shape of a request body.
    #[must_use]
    pub fn of(body: &Value) -> Self {
        let tool_names = body
            .get("tools")
            .and_then(Value::as_array)
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let messages = body.get("messages").and_then(Value::as_array);
        let starts_prompt = messages
            .and_then(|messages| messages.last())
            .is_some_and(is_typed_user_turn);
        Self {
            model: body.get("model").and_then(Value::as_str).map(str::to_owned),
            tool_names,
            has_thinking: body.get("thinking").is_some_and(|t| !t.is_null()),
            message_count: messages.map_or(0, Vec::len),
            stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
            starts_prompt,
        }
    }
}

/// A user message with text and no `tool_result` block.
fn is_typed_user_turn(message: &Value) -> bool {
    if message.get("role").and_then(Value::as_str) != Some("user") {
        return false;
    }
    match message.get("content") {
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(blocks)) => {
            let kind = |block: &Value| block.get("type").and_then(Value::as_str).map(str::to_owned);
            blocks.iter().any(|b| kind(b).as_deref() == Some("text"))
                && !blocks
                    .iter()
                    .any(|b| kind(b).as_deref() == Some("tool_result"))
        }
        _ => false,
    }
}

/// Who sent a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Origin {
    /// The main conversation. Its model is never changed.
    Main,
    /// A subagent (identified by `x-claude-code-agent-id`).
    Subagent {
        /// The agent id.
        agent_id: String,
    },
    /// A side request such as the session title.
    Auxiliary,
    /// The request that summarizes the conversation.
    Compaction,
}

impl Origin {
    /// Short name for records (`main`, `subagent`, `auxiliary`, `compaction`).
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Subagent { .. } => "subagent",
            Self::Auxiliary => "auxiliary",
            Self::Compaction => "compaction",
        }
    }
}

/// Classifies a request (spec 5.1 and the F7 spike results).
///
/// 1. `agent-id` present: a subagent.
/// 2. Compaction headers, or `request-class: compaction`: compaction.
/// 3. `request-class: auxiliary`: auxiliary.
/// 4. No tools, no thinking and exactly one message: auxiliary (the session-title shape). The
///    main loop always carries tools, so it can never match.
/// 5. Anything else: the main loop.
#[must_use]
pub fn classify_origin(meta: &RequestMeta, shape: &RequestShape) -> Origin {
    if let Some(agent_id) = &meta.agent_id {
        return Origin::Subagent {
            agent_id: agent_id.clone(),
        };
    }
    let class = meta.request_class.as_deref().map(str::to_ascii_lowercase);
    if meta.compaction.is_some() || class.as_deref() == Some("compaction") {
        return Origin::Compaction;
    }
    if class.as_deref() == Some("auxiliary") {
        return Origin::Auxiliary;
    }
    if shape.tool_names.is_empty() && !shape.has_thinking && shape.message_count == 1 {
        return Origin::Auxiliary;
    }
    Origin::Main
}

/// Permission to rewrite the model of one request. It cannot be built outside this module:
/// the only way to get one is [`rewrite_ticket`], which refuses the main loop and compaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteTicket {
    _private: (),
}

/// A ticket for a subagent or auxiliary request; `None` for the main loop and compaction.
#[must_use]
pub fn rewrite_ticket(origin: &Origin) -> Option<RewriteTicket> {
    match origin {
        Origin::Subagent { .. } | Origin::Auxiliary => Some(RewriteTicket { _private: () }),
        Origin::Main | Origin::Compaction => None,
    }
}

/// Sets the `model` of a request body. Requires a [`RewriteTicket`].
pub fn rewrite_model(body: &mut Value, _ticket: &RewriteTicket, model: &str) {
    if let Some(object) = body.as_object_mut() {
        object.insert("model".to_owned(), Value::String(model.to_owned()));
    }
}

/// What a hook changed on a request, carried to `after` and to the records.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct HookNotes {
    /// Id of the routing decision applied or observed, if any.
    pub decision_id: Option<String>,
    /// The model the request was rewritten to, if it was.
    pub model_rewritten: Option<String>,
    /// Text tool results replaced by reversible compression in this request.
    pub blocks_compressed: u32,
}

/// One request flowing through the hook pipeline.
#[derive(Debug)]
pub struct RequestCtx<'a> {
    /// Header metadata.
    pub meta: &'a RequestMeta,
    /// Body shape (computed once, from the original body).
    pub shape: &'a RequestShape,
    /// Who sent it.
    pub origin: &'a Origin,
    /// When it arrived.
    pub now: Timestamp,
    /// The body. Hooks may change it; they must then set `changed`.
    pub body: Value,
    /// The body was changed; the proxy re-serializes it instead of forwarding the original bytes.
    pub changed: bool,
    /// What the hooks did.
    pub notes: HookNotes,
}

/// A hook could not process a request. The proxy then forwards the **original** body
/// unchanged (fail-open) and records the failure.
#[derive(Debug, thiserror::Error)]
#[error("{hook}: {message}")]
pub struct HookFailure {
    /// The hook that failed.
    pub hook: &'static str,
    /// What went wrong.
    pub message: String,
}

/// What the upstream answered, as observed by the proxy (never contents).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ResponseSummary {
    /// HTTP status.
    pub status: u16,
    /// `model` reported in the response.
    pub model: Option<String>,
    /// `message.id`.
    pub message_id: Option<String>,
    /// The `request-id` response header.
    pub request_id: Option<String>,
    /// Token usage, including the 1-hour cache split when reported.
    pub usage: Usage,
    /// Thinking tokens, when the API reports them separately.
    pub thinking_tokens: u64,
    /// `stop_reason`.
    pub stop_reason: Option<String>,
    /// Names of the tools the model called in this response, in order.
    pub tool_uses: Vec<String>,
    /// Milliseconds from receiving the request to the first upstream byte.
    pub ttfb_ms: u64,
    /// Milliseconds from receiving the request to the end of the response.
    pub total_ms: u64,
    /// The stream ended early or the body could not be read completely.
    pub interrupted: bool,
}

/// A step of the proxy's pipeline (routing, compression, prompt tracking...). Called on the
/// request path, so implementations must be fast and must not block on I/O: anything durable
/// goes through a [`RecordSink`].
pub trait ProxyHook: Send + Sync {
    /// Short name for logs and failures.
    fn name(&self) -> &'static str;

    /// Before forwarding. May change `ctx.body` (and must then set `ctx.changed`).
    fn before(&self, ctx: &mut RequestCtx<'_>) -> Result<(), HookFailure>;

    /// After the response was fully delivered to the client.
    fn after(&self, _ctx: &RequestCtx<'_>, _response: &ResponseSummary) {}

    /// The request was changed, the upstream rejected it before any byte reached the client,
    /// and the original was sent instead (with `status` being the rejection's status).
    fn fallback(&self, _ctx: &RequestCtx<'_>, _status: u16) {}
}

/// One row of `proxy_requests`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProxyRequestRecord {
    /// Unique id of the request (generated by the proxy).
    pub id: String,
    /// When it arrived.
    pub ts: Timestamp,
    /// Session id (empty when unknown).
    pub session_id: String,
    /// Agent id for subagent requests.
    pub agent_id: Option<String>,
    /// Agent type, when sent.
    pub agent_type: Option<String>,
    /// Prompt id, when sent.
    pub prompt_id: Option<String>,
    /// [`Origin::name`].
    pub origin: String,
    /// The model Claude Code asked for.
    pub model_requested: String,
    /// The model that answered.
    pub model_effective: String,
    /// Usage of the response.
    pub usage: Usage,
    /// Thinking tokens, when reported.
    pub thinking_tokens: u64,
    /// `"<message.id>:<request-id>"`, the key transcripts use, to avoid counting a message twice.
    pub message_key: Option<String>,
    /// Bytes of the request body received.
    pub bytes_in: u64,
    /// Bytes of the request body sent upstream.
    pub bytes_out: u64,
    /// Milliseconds to the first upstream byte.
    pub upstream_ttfb_ms: u64,
    /// Milliseconds to the end of the response.
    pub total_ms: u64,
    /// Microseconds the proxy spent before sending the request upstream.
    pub proxy_overhead_us: u64,
    /// HTTP status returned to the client.
    pub status: u16,
    /// What altered the request, if anything.
    pub notes: HookNotes,
    /// Likely cause of a main-loop cache miss (spec 6.5), when there was one.
    pub miss_cause: Option<String>,
    /// blake3 of the `tools` block.
    pub tools_hash: String,
    /// blake3 of the `system` block.
    pub system_hash: String,
    /// `stop_reason` of the response (`end_turn`, `tool_use`, ...), when there was one.
    pub stop_reason: Option<String>,
    /// Names of the tools offered in the request (at most [`MAX_TOOLS_RECORDED`]); never inputs.
    pub tools: Vec<String>,
    /// Longest cache lifetime the request asked for: `"5m"`, `"1h"`, or `None` when it used no
    /// `cache_control`.
    pub cache_ttl: Option<String>,
    /// `[routing] mode` in force for this request (`off`, `observe` or `enforce`); `None` for
    /// rows recorded before it was stored.
    pub routing_mode: Option<String>,
}

/// Tool names kept per request record.
pub const MAX_TOOLS_RECORDED: usize = 200;

impl ProxyRequestRecord {
    /// The tool names as the JSON array text stored in the database (`[]` when none).
    #[must_use]
    pub fn tools_json(&self) -> String {
        serde_json::to_string(&self.tools).unwrap_or_else(|_| "[]".to_owned())
    }

    /// Reads the text written by [`Self::tools_json`]; anything else is an empty list.
    #[must_use]
    pub fn tools_from_json(text: &str) -> Vec<String> {
        serde_json::from_str(text).unwrap_or_default()
    }
}

/// Read access to the stored proxy requests (`proxy_requests`), for the reports.
pub trait ProxyRequestRepo {
    /// Requests with `from <= ts < to`, oldest first.
    fn requests_between(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<ProxyRequestRecord>, StoreError>;
}

/// Something the proxy wants stored. Written by a single writer, off the request path.
#[derive(Debug, Clone, PartialEq)]
pub enum ProxyRecord {
    /// A request and its response.
    Request(Box<ProxyRequestRecord>),
    /// A routing decision.
    Decision(Box<RoutingDecision>),
    /// A finished user prompt of the main loop.
    Prompt(Box<PromptSummary>),
    /// A model suggestion for the main loop.
    Suggestion(Box<SuggestionRecord>),
    /// What a shadow-mode feature would have saved.
    Shadow(Box<ShadowRecord>),
    /// An original tool result kept for `tokred_retrieve`.
    Stored(Box<StoredResult>),
    /// The bytes sent for a replaced tool result in a session (keeps every turn identical).
    Compressed(Box<CompressedBlock>),
    /// What the dashboard shows of a turn besides its request record (stop reason, tools and,
    /// when `[proxy] log_bodies` is on, the message contents).
    Captured(Box<CapturedTurn>),
}

/// Destination of [`ProxyRecord`]s. Must not block the caller.
pub trait RecordSink: Send + Sync {
    /// Queues a record for storage; failures are logged, never returned.
    fn record(&self, record: ProxyRecord);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn shape(body: &Value) -> RequestShape {
        RequestShape::of(body)
    }

    #[test]
    fn headers_are_read_case_insensitively_and_empty_values_ignored() {
        let meta = RequestMeta::from_headers([
            ("X-Claude-Code-Session-Id", "s1"),
            ("x-claude-code-agent-id", "a1"),
            ("x-claude-code-parent-agent-id", "p"),
            ("x-claude-code-agent-type", "Explore"),
            ("x-claude-code-request-class", "subagent"),
            ("x-claude-code-prompt-id", "pr"),
            ("x-claude-code-compaction", "auto"),
            ("x-claude-code-context-compacted", "manual"),
            ("x-claude-code-unknown", "x"),
            ("authorization", "secret"),
            ("x-claude-code-agent-type", " "),
            (
                "Anthropic-Beta",
                "oauth-2025-04-20, context-1m-2025-08-07,,",
            ),
            ("anthropic-beta", "effort-2025-11-24"),
        ]);
        assert_eq!(
            meta.betas,
            [
                "oauth-2025-04-20",
                "context-1m-2025-08-07",
                "effort-2025-11-24"
            ]
        );
        assert_eq!(meta.session_id.as_deref(), Some("s1"));
        assert_eq!(meta.agent_id.as_deref(), Some("a1"));
        assert_eq!(meta.parent_agent_id.as_deref(), Some("p"));
        assert_eq!(
            meta.agent_type.as_deref(),
            Some("Explore"),
            "blank value ignored"
        );
        assert_eq!(meta.request_class.as_deref(), Some("subagent"));
        assert_eq!(meta.prompt_id.as_deref(), Some("pr"));
        assert_eq!(meta.compaction.as_deref(), Some("auto"));
        assert_eq!(meta.context_compacted.as_deref(), Some("manual"));
    }

    #[test]
    fn the_shape_reads_structure_only() {
        let body = json!({
            "model": "claude-sonnet-5-5",
            "stream": true,
            "thinking": {"type": "adaptive"},
            "tools": [{"name": "Read"}, {"name": "Grep"}, {"other": 1}],
            "messages": [
                {"role": "user", "content": "find the file"},
                {"role": "assistant", "content": [{"type": "tool_use"}]},
                {"role": "user", "content": [{"type": "tool_result", "content": "x"}]}
            ]
        });
        let s = shape(&body);
        assert_eq!(s.model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(s.tool_names, ["Read", "Grep"]);
        assert!(s.has_thinking && s.stream);
        assert_eq!(s.message_count, 3);
        assert!(!s.starts_prompt, "a tool result continues the loop");

        let typed = json!({"messages": [{"role": "user", "content": [
            {"type": "text", "text": "<system-reminder>…</system-reminder>"},
            {"type": "text", "text": "now add tests"}
        ]}]});
        assert!(shape(&typed).starts_prompt);
        assert!(shape(&json!({"messages": [{"role": "user", "content": "hi"}]})).starts_prompt);
        for not_typed in [
            json!({"messages": [{"role": "user", "content": ""}]}),
            json!({"messages": [{"role": "assistant", "content": "hi"}]}),
            json!({"messages": [{"role": "user", "content": 3}]}),
            json!({"messages": []}),
            json!({"thinking": null}),
            json!([]),
        ] {
            let s = shape(&not_typed);
            assert!(!s.starts_prompt && !s.has_thinking, "{not_typed}");
        }
    }

    fn meta_with(
        agent: Option<&str>,
        class: Option<&str>,
        compaction: Option<&str>,
    ) -> RequestMeta {
        RequestMeta {
            agent_id: agent.map(str::to_owned),
            request_class: class.map(str::to_owned),
            compaction: compaction.map(str::to_owned),
            ..RequestMeta::default()
        }
    }

    fn main_loop_shape() -> RequestShape {
        RequestShape {
            tool_names: vec!["Read".to_owned()],
            has_thinking: true,
            message_count: 4,
            ..RequestShape::default()
        }
    }

    fn title_shape() -> RequestShape {
        RequestShape {
            message_count: 1,
            ..RequestShape::default()
        }
    }

    #[test]
    fn origins_follow_the_documented_rules() {
        assert_eq!(
            classify_origin(
                &meta_with(Some("a1"), Some("main"), None),
                &main_loop_shape()
            ),
            Origin::Subagent {
                agent_id: "a1".to_owned()
            },
            "agent-id wins"
        );
        assert_eq!(
            classify_origin(&meta_with(None, None, Some("auto")), &title_shape()),
            Origin::Compaction
        );
        assert_eq!(
            classify_origin(
                &meta_with(None, Some("Compaction"), None),
                &main_loop_shape()
            ),
            Origin::Compaction
        );
        assert_eq!(
            classify_origin(
                &meta_with(None, Some("auxiliary"), None),
                &main_loop_shape()
            ),
            Origin::Auxiliary
        );
        assert_eq!(
            classify_origin(&RequestMeta::default(), &title_shape()),
            Origin::Auxiliary,
            "the session-title shape"
        );
        assert_eq!(
            classify_origin(&RequestMeta::default(), &main_loop_shape()),
            Origin::Main
        );
        // Near misses of the title shape stay main.
        for shape in [
            RequestShape {
                message_count: 2,
                ..title_shape()
            },
            RequestShape {
                has_thinking: true,
                ..title_shape()
            },
            RequestShape {
                tool_names: vec!["Read".to_owned()],
                ..title_shape()
            },
        ] {
            assert_eq!(
                classify_origin(&RequestMeta::default(), &shape),
                Origin::Main
            );
        }
    }

    #[test]
    fn only_subagent_and_auxiliary_requests_can_be_rewritten() {
        assert!(rewrite_ticket(&Origin::Main).is_none());
        assert!(rewrite_ticket(&Origin::Compaction).is_none());
        let ticket = rewrite_ticket(&Origin::Auxiliary).unwrap();
        assert!(
            rewrite_ticket(&Origin::Subagent {
                agent_id: "a".to_owned()
            })
            .is_some()
        );

        let mut body = json!({"model": "claude-sonnet-5-5", "messages": []});
        rewrite_model(&mut body, &ticket, "claude-haiku-5-5");
        assert_eq!(body["model"], "claude-haiku-5-5");
        let mut not_an_object = json!([1]);
        rewrite_model(&mut not_an_object, &ticket, "x");
        assert_eq!(not_an_object, json!([1]));
    }

    #[test]
    fn tool_names_round_trip_through_their_json_text() {
        let record = ProxyRequestRecord {
            tools: vec!["Read".to_owned(), "mcp__x__y".to_owned()],
            ..ProxyRequestRecord::default()
        };
        assert_eq!(record.tools_json(), r#"["Read","mcp__x__y"]"#);
        assert_eq!(
            ProxyRequestRecord::tools_from_json(&record.tools_json()),
            record.tools
        );
        assert_eq!(ProxyRequestRecord::default().tools_json(), "[]");
        assert!(ProxyRequestRecord::tools_from_json("not json").is_empty());
    }

    #[test]
    fn origin_names() {
        assert_eq!(Origin::Main.name(), "main");
        assert_eq!(
            Origin::Subagent {
                agent_id: String::new()
            }
            .name(),
            "subagent"
        );
        assert_eq!(Origin::Auxiliary.name(), "auxiliary");
        assert_eq!(Origin::Compaction.name(), "compaction");
    }

    #[test]
    fn hook_failures_name_the_hook() {
        let failure = HookFailure {
            hook: "routing",
            message: "bad body".to_owned(),
        };
        assert_eq!(failure.to_string(), "routing: bad body");
    }
}
