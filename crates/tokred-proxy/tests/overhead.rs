//! Proxy overhead (spec 3.3 and 8.4): time from receiving a request to sending it upstream,
//! with a realistic main-loop body, must stay under 10 ms at the 95th percentile. Debug builds
//! (plain `cargo test`, coverage) are only held to a loose bound; the budget applies to release
//! builds: `cargo test --release -p tokred-proxy --test overhead`.
//!
//! Set `TOKRED_PROXY_OVERHEAD_REPORT=<file>` to write the measured percentiles to a file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use serde_json::json;
use support::{Behavior, FakeUpstream, Reply, hook, post_messages, sse_body, start};

const REQUESTS: usize = 200;
const BUDGET_US: u64 = 10_000;
const DEBUG_BOUND_US: u64 = 200_000;

/// About 300 KB: 15 tools with schemas, a 25 KB system prompt and 150 messages.
fn realistic_body() -> Vec<u8> {
    let tools: Vec<_> = (0..15)
        .map(|i| {
            json!({
                "name": format!("Tool{i}"),
                "description": "Reads, searches or edits files in the workspace. ".repeat(30),
                "input_schema": {"type": "object", "properties": {
                    "path": {"type": "string", "description": "Absolute path"},
                    "limit": {"type": "integer"}, "offset": {"type": "integer"}
                }, "required": ["path"]}
            })
        })
        .collect();
    let messages: Vec<_> = (0..150)
        .map(|i| {
            if i % 2 == 0 {
                json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": format!("t{i}"),
                    "content": "fn main() { println!(\"hello\"); }\n".repeat(80)}]})
            } else {
                json!({"role": "assistant", "content": [
                    {"type": "text", "text": "Let me look at that file."},
                    {"type": "tool_use", "id": format!("t{i}"), "name": "Read", "input": {"path": "/src/main.rs"}}]})
            }
        })
        .collect();
    json!({
        "model": "claude-opus-5-5",
        "max_tokens": 32000,
        "stream": true,
        "thinking": {"type": "adaptive"},
        "output_config": {"effort": "high"},
        "system": [
            {"type": "text", "text": "x-anthropic-billing-header: cc_version=2.1.312"},
            {"type": "text", "text": "You are Claude Code. ".repeat(1200), "cache_control": {"type": "ephemeral", "ttl": "1h"}}
        ],
        "tools": tools,
        "messages": messages
    })
    .to_string()
    .into_bytes()
}

fn percentile(sorted: &[u64], percent: usize) -> u64 {
    sorted[(sorted.len() - 1) * percent / 100]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_p95_overhead_stays_within_budget() {
    let body = realistic_body();
    assert!(body.len() > 250_000, "body is {} bytes", body.len());
    let upstream =
        FakeUpstream::always(Reply::stream(vec![sse_body("msg_o", "claude-opus-5-5")])).await;
    let (observer, _) = hook(Behavior::Observe);
    let proxy = start(&upstream.url(), vec![observer]).await;
    let headers = [("x-claude-code-session-id", "bench")];
    for _ in 0..REQUESTS {
        let answer = post_messages(proxy.addr, &headers, &body).await;
        assert_eq!(answer.status, 200);
    }
    let mut overheads: Vec<u64> = proxy
        .sink
        .wait_for(REQUESTS)
        .await
        .iter()
        .map(|r| r.proxy_overhead_us)
        .collect();
    overheads.sort_unstable();
    let (p50, p95, max) = (
        percentile(&overheads, 50),
        percentile(&overheads, 95),
        overheads[overheads.len() - 1],
    );
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    if let Ok(path) = std::env::var("TOKRED_PROXY_OVERHEAD_REPORT") {
        std::fs::write(
            path,
            format!(
                "profile={profile} body_bytes={} requests={REQUESTS} p50_us={p50} p95_us={p95} max_us={max}\n",
                body.len()
            ),
        )
        .unwrap();
    }
    let bound = if cfg!(debug_assertions) {
        DEBUG_BOUND_US
    } else {
        BUDGET_US
    };
    assert!(
        p95 < bound,
        "{profile} p95 overhead {p95} us (p50 {p50}, max {max})"
    );
    proxy.stop().await;
}
