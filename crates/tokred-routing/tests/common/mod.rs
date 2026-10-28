//! Shared helpers: a recording sink, a price table and request builders.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokred_core::Timestamp;
use tokred_core::config::{RoutingConfig, TrafficMode};
use tokred_core::pricing::{ModelPrice, PriceEntry, PriceTable};
use tokred_core::proxy::{
    HookNotes, ProxyHook, ProxyRecord, RecordSink, RequestCtx, RequestMeta, RequestShape,
    classify_origin,
};
use tokred_core::routing::RoutingDecision;
use tokred_routing::RoutingHook;

/// Keeps every record.
#[derive(Default)]
pub struct Sink(pub Mutex<Vec<ProxyRecord>>);

impl RecordSink for Sink {
    fn record(&self, record: ProxyRecord) {
        self.0.lock().unwrap().push(record);
    }
}

impl Sink {
    pub fn decisions(&self) -> Vec<RoutingDecision> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|record| match record {
                ProxyRecord::Decision(decision) => (**decision).clone(),
                other => panic!("unexpected record {other:?}"),
            })
            .collect()
    }
}

/// Prices in USD per million tokens (the embedded snapshot's values).
pub fn prices() -> PriceTable {
    let row = |model: &str, input: f64, output: f64, read: f64| PriceEntry {
        model: model.to_owned(),
        price: ModelPrice {
            input,
            output,
            cache_write: input * 1.25,
            cache_write_1h: input * 2.0,
            cache_read: read,
        },
        source: "test".to_owned(),
        updated_at: Timestamp(0),
    };
    PriceTable::new(vec![
        row("claude-opus-5-5", 4.0, 20.0, 0.2),
        row("claude-opus-5", 5.0, 25.0, 0.5),
        row("claude-opus-4-8", 5.0, 25.0, 0.5),
        row("claude-opus-4-6", 5.0, 25.0, 0.5),
        row("claude-sonnet-5-5", 2.0, 10.0, 0.1),
        row("claude-sonnet-5", 2.0, 10.0, 0.2),
        row("claude-sonnet-4-6", 3.0, 15.0, 0.3),
        row("claude-haiku-5-5", 0.1, 0.5, 0.01),
        row("claude-haiku-4-5", 1.0, 5.0, 0.1),
        row("claude-fable-5-1", 10.0, 50.0, 0.25),
    ])
}

pub fn config(mode: TrafficMode) -> RoutingConfig {
    RoutingConfig {
        mode,
        ..RoutingConfig::default()
    }
}

pub fn new_hook_with(config: RoutingConfig) -> (RoutingHook, Arc<Sink>) {
    let sink = Arc::new(Sink::default());
    (RoutingHook::new(config, prices(), sink.clone()), sink)
}

pub fn new_hook(mode: TrafficMode) -> (RoutingHook, Arc<Sink>) {
    new_hook_with(config(mode))
}

/// A request as the proxy sees it.
#[derive(Debug, Clone)]
pub struct Req {
    pub meta: RequestMeta,
    pub body: Value,
    pub now: i64,
}

/// What the hook left on the request.
#[derive(Debug, Clone)]
pub struct Done {
    pub body: Value,
    pub changed: bool,
    pub notes: HookNotes,
}

pub fn run(hook: &RoutingHook, req: &Req) -> Done {
    let shape = RequestShape::of(&req.body);
    let origin = classify_origin(&req.meta, &shape);
    let mut ctx = RequestCtx {
        meta: &req.meta,
        shape: &shape,
        origin: &origin,
        now: Timestamp(req.now),
        body: req.body.clone(),
        changed: false,
        notes: HookNotes::default(),
    };
    hook.before(&mut ctx).unwrap();
    Done {
        body: ctx.body,
        changed: ctx.changed,
        notes: ctx.notes,
    }
}

/// Reports to the hook that the upstream rejected the rewritten request.
pub fn reject(hook: &RoutingHook, req: &Req, done: &Done, status: u16) {
    let shape = RequestShape::of(&req.body);
    let origin = classify_origin(&req.meta, &shape);
    let ctx = RequestCtx {
        meta: &req.meta,
        shape: &shape,
        origin: &origin,
        now: Timestamp(req.now),
        body: done.body.clone(),
        changed: done.changed,
        notes: done.notes.clone(),
    };
    hook.fallback(&ctx, status);
    hook.after(&ctx, &tokred_core::proxy::ResponseSummary::default());
}

pub fn meta(session: Option<&str>, agent: Option<&str>) -> RequestMeta {
    RequestMeta {
        session_id: session.map(str::to_owned),
        agent_id: agent.map(str::to_owned),
        ..RequestMeta::default()
    }
}

fn tools(names: &[&str]) -> Value {
    Value::Array(
        names
            .iter()
            .map(|name| json!({"name": name, "input_schema": {"type": "object"}}))
            .collect(),
    )
}

/// A main-loop turn.
pub fn main_turn(session: &str, model: &str, now: i64) -> Req {
    Req {
        meta: meta(Some(session), None),
        body: json!({
            "model": model, "max_tokens": 32000, "stream": true,
            "thinking": {"type": "adaptive"},
            "system": [{"type": "text", "text": "main"}],
            "tools": tools(&["Read", "Edit", "Bash", "Agent"]),
            "messages": [{"role": "user", "content": "hi"}]
        }),
        now,
    }
}

/// The first request of a subagent, with a large system prompt (as Claude Code sends).
pub fn first_turn(
    session: &str,
    agent: &str,
    model: &str,
    tool_names: &[&str],
    prompt: &str,
    now: i64,
) -> Req {
    Req {
        meta: meta(Some(session), Some(agent)),
        body: json!({
            "model": model, "max_tokens": 32000, "stream": true,
            "thinking": {"type": "adaptive"},
            "system": [{"type": "text", "text": "x".repeat(20_000)}],
            "tools": tools(tool_names),
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "<system-reminder>architecture and plan rules</system-reminder>"},
                {"type": "text", "text": prompt}
            ]}]
        }),
        now,
    }
}

/// A later request of the same subagent (a tool result goes back).
pub fn later_turn(first: &Req, now: i64) -> Req {
    let mut body = first.body.clone();
    let messages = body["messages"].as_array_mut().unwrap();
    messages.push(json!({"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "Read", "input": {}}]}));
    messages.push(json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "file"}]}));
    Req {
        meta: first.meta.clone(),
        body,
        now,
    }
}

/// An auxiliary request with the session-title shape.
pub fn title(session: &str, model: &str, text_len: usize, now: i64) -> Req {
    Req {
        meta: meta(Some(session), None),
        body: json!({
            "model": model, "max_tokens": 512,
            "system": "Write a short title for the conversation.",
            "messages": [{"role": "user", "content": "y".repeat(text_len)}]
        }),
        now,
    }
}

pub const SONNET: &str = "claude-sonnet-5-5";
pub const HAIKU: &str = "claude-haiku-5-5";
pub const OPUS: &str = "claude-opus-5-5";
pub const READ_ONLY: &[&str] = &["Read", "Grep", "Glob"];
pub const SEARCH_PROMPT: &str = "Look through the repository and report how sessions expire";
