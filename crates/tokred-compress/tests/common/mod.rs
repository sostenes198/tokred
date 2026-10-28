//! Helpers shared by the integration tests: a recording sink, a request runner and a simulated
//! Claude Code conversation.
#![allow(dead_code, clippy::needless_pass_by_value)]

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokred_compress::CompressionHook;
use tokred_core::Timestamp;
use tokred_core::compression::{CompressedBlock, ShadowRecord, StoredResult};
use tokred_core::config::{CompressionConfig, ShadowConfig, TrafficMode};
use tokred_core::proxy::{
    HookNotes, ProxyHook, ProxyRecord, RecordSink, RequestCtx, RequestMeta, RequestShape,
    classify_origin,
};

/// Keeps every record it receives.
#[derive(Debug, Default)]
pub struct Sink(Mutex<Vec<ProxyRecord>>);

impl RecordSink for Sink {
    fn record(&self, record: ProxyRecord) {
        self.0.lock().unwrap().push(record);
    }
}

impl Sink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn all(&self) -> Vec<ProxyRecord> {
        self.0.lock().unwrap().clone()
    }

    pub fn stored(&self) -> Vec<StoredResult> {
        self.all()
            .into_iter()
            .filter_map(|r| match r {
                ProxyRecord::Stored(s) => Some(*s),
                _ => None,
            })
            .collect()
    }

    pub fn compressed(&self) -> Vec<CompressedBlock> {
        self.all()
            .into_iter()
            .filter_map(|r| match r {
                ProxyRecord::Compressed(c) => Some(*c),
                _ => None,
            })
            .collect()
    }

    pub fn shadow(&self) -> Vec<ShadowRecord> {
        self.all()
            .into_iter()
            .filter_map(|r| match r {
                ProxyRecord::Shadow(s) => Some(*s),
                _ => None,
            })
            .collect()
    }
}

pub fn config(mode: TrafficMode) -> CompressionConfig {
    CompressionConfig {
        mode,
        ..CompressionConfig::default()
    }
}

/// A hook with the default rules in `mode`, the MCP server registered or not.
pub fn hook(mode: TrafficMode, mcp_registered: bool, sink: &Arc<Sink>) -> CompressionHook {
    CompressionHook::new(
        &config(mode),
        &ShadowConfig::default(),
        mcp_registered,
        Arc::clone(sink) as Arc<dyn RecordSink>,
    )
}

pub fn meta(session: Option<&str>, agent: Option<&str>) -> RequestMeta {
    RequestMeta {
        session_id: session.map(str::to_owned),
        agent_id: agent.map(str::to_owned),
        ..RequestMeta::default()
    }
}

/// What a hook did to one request.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub body: Value,
    pub changed: bool,
    pub blocks: u32,
}

pub fn run(hook: &dyn ProxyHook, meta: &RequestMeta, body: Value, now: i64) -> Outcome {
    let shape = RequestShape::of(&body);
    let origin = classify_origin(meta, &shape);
    let mut ctx = RequestCtx {
        meta,
        shape: &shape,
        origin: &origin,
        now: Timestamp(now),
        body,
        changed: false,
        notes: HookNotes::default(),
    };
    hook.before(&mut ctx).unwrap();
    Outcome {
        body: ctx.body,
        changed: ctx.changed,
        blocks: ctx.notes.blocks_compressed,
    }
}

/// A large tool output: `lines` lines tagged with `tag`, well above the default threshold.
pub fn big(tag: &str, lines: usize) -> String {
    (0..lines)
        .map(|n| format!("{tag}: output line {n:05} with some padding text\n"))
        .collect::<Vec<_>>()
        .concat()
}

/// A Claude Code conversation as resent on every turn (always the originals).
#[derive(Debug, Clone)]
pub struct Conversation {
    pub messages: Vec<Value>,
    next_id: usize,
    /// Move the `cache_control` breakpoint to the last block of the last message, as Claude
    /// Code does.
    pub move_breakpoint: bool,
}

impl Conversation {
    pub fn new(prompt: &str) -> Self {
        Self {
            messages: vec![json!({"role": "user", "content": [{"type": "text", "text": prompt}]})],
            next_id: 0,
            move_breakpoint: false,
        }
    }

    /// The model calls `tool` and the result comes back with `content`; returns the `tool_use` id.
    pub fn tool_turn(&mut self, tool: &str, content: Value) -> String {
        self.next_id += 1;
        let id = format!("toolu_{:03}", self.next_id);
        self.messages.push(json!({"role": "assistant", "content": [
            {"type": "thinking", "thinking": "plan", "signature": "sig"},
            {"type": "text", "text": format!("Running {tool}.")},
            {"type": "tool_use", "id": id, "name": tool, "input": {"command": "x"}}
        ]}));
        self.messages.push(json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": id, "content": content}
        ]}));
        id
    }

    /// The model calls `tokred_retrieve(id)` and gets `answer` back.
    pub fn retrieve_turn(&mut self, retrieve_id: &str, answer: &str) {
        self.next_id += 1;
        let id = format!("toolu_{:03}", self.next_id);
        self.messages.push(json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": id, "name": "mcp__tokred__tokred_retrieve",
             "input": {"id": retrieve_id, "offset": 40}}
        ]}));
        self.messages.push(json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": id, "content": answer}
        ]}));
    }

    /// The request body Claude Code sends now (`messages` last, so serialized bodies of
    /// consecutive turns share a byte prefix).
    pub fn body(&self) -> Value {
        let mut messages = self.messages.clone();
        if self.move_breakpoint
            && let Some(block) = messages
                .last_mut()
                .and_then(|m| m["content"].as_array_mut())
                .and_then(|blocks| blocks.last_mut())
        {
            block["cache_control"] = json!({"type": "ephemeral"});
        }
        json!({
            "model": "claude-opus-5-5",
            "max_tokens": 32000,
            "stream": true,
            "thinking": {"type": "adaptive"},
            "system": [{"type": "text", "text": "System prompt.",
                        "cache_control": {"type": "ephemeral"}}],
            "tools": [{"name": "Bash", "input_schema": {"type": "object"}},
                      {"name": "mcp__tokred__tokred_retrieve", "input_schema": {"type": "object"}}],
            "messages": messages
        })
    }
}

/// The text of the `tool_result` in message `index` (string content).
pub fn result_text(body: &Value, index: usize) -> &str {
    body["messages"][index]["content"][0]["content"]
        .as_str()
        .unwrap()
}
