//! What the proxy reads from a `/v1/messages` response while forwarding it: message id, model,
//! usage (with the 1-hour cache split and thinking tokens), stop reason and the tools called.
//!
//! Field names follow the official streaming and prompt-caching docs: `message_start` carries
//! the message and its initial `usage`; `message_delta` carries `delta.stop_reason` and a
//! **cumulative** `usage` (fields present replace earlier values); thinking tokens are reported
//! as `usage.output_tokens_details.thinking_tokens`, on the final `message_delta` when
//! streaming; the 1-hour cache write is `usage.cache_creation.ephemeral_1h_input_tokens`.

use hyper::HeaderMap;
use hyper::header::{CONTENT_ENCODING, CONTENT_TYPE};
use serde_json::Value;
use tokred_core::pricing::Usage;

use crate::headers::text;
use crate::sse::SseParser;

/// A JSON body larger than this is forwarded but not parsed.
const MAX_JSON_BYTES: usize = 16 * 1024 * 1024;

/// Events whose data is parsed; the rest (deltas, pings) are skipped without parsing.
const EVENTS: [&str; 5] = [
    "message_start",
    "content_block_start",
    "message_delta",
    "message_stop",
    "error",
];

/// How the body is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `text/event-stream`.
    Sse,
    /// `application/json`.
    Json,
    /// Anything else, or a compressed body: forwarded but not read.
    Opaque,
}

/// The facts gathered from one response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Observed {
    /// `message.id`.
    pub message_id: Option<String>,
    /// The model that answered.
    pub model: Option<String>,
    /// Token usage.
    pub usage: Usage,
    /// Thinking tokens (part of the output tokens), when reported.
    pub thinking_tokens: u64,
    /// `stop_reason`.
    pub stop_reason: Option<String>,
    /// Tools called, in order.
    pub tool_uses: Vec<String>,
    /// The body broke off, the stream ended without `message_stop`, or it carried an error.
    pub interrupted: bool,
}

#[derive(Debug, Default)]
struct State {
    observed: Observed,
    saw_stop: bool,
    saw_error: bool,
}

fn set(slot: &mut u64, value: Option<u64>) {
    if let Some(value) = value {
        *slot = value;
    }
}

fn set_text(slot: &mut Option<String>, value: Option<&Value>) {
    if let Some(value) = value.and_then(Value::as_str) {
        *slot = Some(value.to_owned());
    }
}

impl State {
    /// Fields present in a `usage` object replace the current ones (the streaming usage is
    /// cumulative, so the last value wins).
    fn usage(&mut self, usage: Option<&Value>) {
        let Some(usage) = usage else { return };
        let number = |pointer: &str| usage.pointer(pointer).and_then(Value::as_u64);
        let target = &mut self.observed.usage;
        set(&mut target.input, number("/input_tokens"));
        set(&mut target.output, number("/output_tokens"));
        set(
            &mut target.cache_write,
            number("/cache_creation_input_tokens"),
        );
        set(&mut target.cache_read, number("/cache_read_input_tokens"));
        set(
            &mut target.cache_write_1h,
            number("/cache_creation/ephemeral_1h_input_tokens"),
        );
        set(
            &mut self.observed.thinking_tokens,
            number("/output_tokens_details/thinking_tokens"),
        );
    }

    /// A tool call block (`tool_use`, `server_tool_use`, `mcp_tool_use`).
    fn block(&mut self, block: &Value) {
        let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
        if let (true, Some(name)) = (
            kind.ends_with("tool_use"),
            block.get("name").and_then(Value::as_str),
        ) {
            self.observed.tool_uses.push(name.to_owned());
        }
    }

    /// A `Message` object: the whole non-streaming body, or the one in `message_start`.
    fn message(&mut self, message: &Value) {
        set_text(&mut self.observed.message_id, message.get("id"));
        set_text(&mut self.observed.model, message.get("model"));
        set_text(&mut self.observed.stop_reason, message.get("stop_reason"));
        self.usage(message.get("usage"));
        for block in message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            self.block(block);
        }
    }

    fn event(&mut self, name: Option<&str>, data: &[u8]) {
        if name.is_some_and(|name| !EVENTS.contains(&name)) {
            return;
        }
        let Ok(event) = serde_json::from_slice::<Value>(data) else {
            return;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => self.message(event.get("message").unwrap_or(&Value::Null)),
            Some("content_block_start") => {
                self.block(event.get("content_block").unwrap_or(&Value::Null));
            }
            Some("message_delta") => {
                set_text(
                    &mut self.observed.stop_reason,
                    event.pointer("/delta/stop_reason"),
                );
                self.usage(event.get("usage"));
            }
            Some("message_stop") => self.saw_stop = true,
            Some("error") => self.saw_error = true,
            _ => {}
        }
    }
}

/// Observes a response body chunk by chunk.
#[derive(Debug)]
pub(crate) struct ResponseTap {
    kind: Kind,
    sse: SseParser,
    json: Vec<u8>,
    json_overflow: bool,
    state: State,
}

impl ResponseTap {
    /// Chooses how to read the body from the response headers. A compressed body is not read:
    /// the proxy forwards `accept-encoding` unchanged and does not decompress.
    pub(crate) fn new(headers: &HeaderMap) -> Self {
        let encoded = text(headers, CONTENT_ENCODING.as_str())
            .is_some_and(|value| !value.trim().eq_ignore_ascii_case("identity"));
        let content_type = text(headers, CONTENT_TYPE.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let kind = if encoded {
            tracing::debug!("compressed response: usage is not read");
            Kind::Opaque
        } else if content_type.starts_with("text/event-stream") {
            Kind::Sse
        } else if content_type.starts_with("application/json") {
            Kind::Json
        } else {
            Kind::Opaque
        };
        Self {
            kind,
            sse: SseParser::default(),
            json: Vec::new(),
            json_overflow: false,
            state: State::default(),
        }
    }

    /// Reads one chunk of the body.
    pub(crate) fn feed(&mut self, chunk: &[u8]) {
        match self.kind {
            Kind::Sse => {
                let state = &mut self.state;
                self.sse
                    .feed(chunk, &mut |name, data| state.event(name, data));
            }
            Kind::Json if self.json.len() + chunk.len() <= MAX_JSON_BYTES => {
                self.json.extend_from_slice(chunk);
            }
            Kind::Json => self.json_overflow = true,
            Kind::Opaque => {}
        }
    }

    /// Ends the observation. `complete` is false when the body broke off.
    pub(crate) fn finish(mut self, complete: bool) -> Observed {
        if self.kind == Kind::Json
            && !self.json_overflow
            && let Ok(message) = serde_json::from_slice::<Value>(&self.json)
        {
            self.state.message(&message);
        }
        let stream_cut = self.kind == Kind::Sse && !self.state.saw_stop;
        self.state.observed.interrupted = !complete || stream_cut || self.state.saw_error;
        self.state.observed
    }
}

#[cfg(test)]
mod tests {
    use hyper::header::HeaderValue;
    use serde_json::json;

    use super::*;

    fn headers(content_type: &'static str, encoding: Option<&'static str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
        if let Some(encoding) = encoding {
            headers.insert(CONTENT_ENCODING, HeaderValue::from_static(encoding));
        }
        headers
    }

    fn sse(events: &[Value]) -> Vec<u8> {
        let mut out = Vec::new();
        for event in events {
            let name = event["type"].as_str().unwrap();
            out.extend_from_slice(format!("event: {name}\ndata: {event}\n\n").as_bytes());
        }
        out
    }

    fn full_stream() -> Vec<u8> {
        sse(&[
            json!({"type": "message_start", "message": {
                "id": "msg_1", "model": "claude-opus-5-5", "content": [], "stop_reason": null,
                "usage": {"input_tokens": 10, "output_tokens": 1, "cache_creation_input_tokens": 300,
                          "cache_read_input_tokens": 5000,
                          "cache_creation": {"ephemeral_5m_input_tokens": 100, "ephemeral_1h_input_tokens": 200}}}}),
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
            json!({"type": "ping"}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "hm"}}),
            json!({"type": "content_block_start", "index": 1, "content_block": {"type": "tool_use", "id": "t1", "name": "Read", "input": {}}}),
            json!({"type": "content_block_start", "index": 2, "content_block": {"type": "server_tool_use", "id": "t2", "name": "web_search"}}),
            json!({"type": "content_block_start", "index": 3, "content_block": {"type": "tool_use", "id": "t3"}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 50}}),
            json!({"type": "message_delta", "delta": {}, "usage": {"output_tokens": 80, "output_tokens_details": {"thinking_tokens": 30}}}),
            json!({"type": "message_stop"}),
        ])
    }

    fn expected_stream() -> Observed {
        Observed {
            message_id: Some("msg_1".to_owned()),
            model: Some("claude-opus-5-5".to_owned()),
            usage: Usage {
                input: 10,
                output: 80,
                cache_write: 300,
                cache_write_1h: 200,
                cache_read: 5000,
            },
            thinking_tokens: 30,
            stop_reason: Some("tool_use".to_owned()),
            tool_uses: vec!["Read".to_owned(), "web_search".to_owned()],
            interrupted: false,
        }
    }

    #[test]
    fn a_stream_is_read_whatever_the_chunking() {
        let stream = full_stream();
        for size in [1, 2, 3, 7, 64, stream.len()] {
            let mut tap = ResponseTap::new(&headers("text/event-stream; charset=utf-8", None));
            for chunk in stream.chunks(size) {
                tap.feed(chunk);
            }
            assert_eq!(tap.finish(true), expected_stream(), "chunks of {size}");
        }
    }

    #[test]
    fn a_stream_without_message_stop_or_with_an_error_is_interrupted() {
        let stream = full_stream();
        let mut tap = ResponseTap::new(&headers("text/event-stream", None));
        tap.feed(&stream[..stream.len() - 40]);
        let observed = tap.finish(true);
        assert!(observed.interrupted);
        assert_eq!(observed.usage.cache_read, 5000, "what arrived is kept");

        let mut tap = ResponseTap::new(&headers("text/event-stream", None));
        tap.feed(&stream);
        assert!(tap.finish(false).interrupted, "transport error");

        let mut tap = ResponseTap::new(&headers("text/event-stream", None));
        tap.feed(&sse(&[
            json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}),
            json!({"type": "message_stop"}),
        ]));
        assert!(tap.finish(true).interrupted);
    }

    #[test]
    fn unusable_events_are_ignored() {
        let mut tap = ResponseTap::new(&headers("text/event-stream", None));
        tap.feed(b"event: message_start\ndata: not json\n\n");
        tap.feed(b"data: {\"type\":\"message_start\"}\n\n");
        tap.feed(b"data: {\"type\":\"content_block_start\"}\n\n");
        tap.feed(b"data: {\"type\":\"surprise\"}\n\n");
        tap.feed(b"data: {\"type\":\"message_stop\"}\n\n");
        assert_eq!(tap.finish(true), Observed::default());
    }

    #[test]
    fn a_json_message_is_read_at_the_end() {
        let body = json!({
            "id": "msg_2", "type": "message", "model": "claude-haiku-5-5", "stop_reason": "end_turn",
            "content": [{"type": "text", "text": "hi"}, {"type": "tool_use", "name": "Grep", "id": "x", "input": {}}],
            "usage": {"input_tokens": 3, "output_tokens": 4}
        })
        .to_string();
        let mut tap = ResponseTap::new(&headers("application/json", Some("identity")));
        let (a, b) = body.as_bytes().split_at(10);
        tap.feed(a);
        tap.feed(b);
        let observed = tap.finish(true);
        assert_eq!(observed.message_id.as_deref(), Some("msg_2"));
        assert_eq!(observed.model.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(observed.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(observed.tool_uses, ["Grep"]);
        assert_eq!(observed.usage.input + observed.usage.output, 7);
        assert!(!observed.interrupted);
    }

    #[test]
    fn error_and_oversized_json_bodies_give_no_usage() {
        let mut tap = ResponseTap::new(&headers("application/json", None));
        tap.feed(br#"{"type":"error","error":{"type":"rate_limit_error","message":"x"}}"#);
        assert_eq!(tap.finish(true), Observed::default());

        let mut tap = ResponseTap::new(&headers("application/json", None));
        tap.feed(b"{\"id\":\"m\"");
        tap.feed(&vec![b' '; MAX_JSON_BYTES]);
        tap.feed(b"}");
        assert_eq!(tap.finish(false).message_id, None);
    }

    #[test]
    fn compressed_or_unknown_bodies_are_not_read() {
        let stream = full_stream();
        for headers in [
            headers("text/event-stream", Some("gzip")),
            headers("text/plain", None),
            HeaderMap::new(),
        ] {
            let mut tap = ResponseTap::new(&headers);
            tap.feed(&stream);
            assert_eq!(tap.finish(true), Observed::default());
        }
    }
}
