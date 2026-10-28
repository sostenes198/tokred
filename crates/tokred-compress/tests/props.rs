//! Properties of the compression hook over generated conversations.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{Conversation, Sink, hook, meta, run};
use proptest::prelude::*;
use serde_json::{Value, json};
use tokred_compress::ResponseCacheShadow;
use tokred_core::config::TrafficMode;
use tokred_core::proxy::RecordSink;

/// One tool result: its tool, its content and whether it is an error.
fn result() -> impl Strategy<Value = (String, Value, bool)> {
    let tool = prop::sample::select(vec!["Bash", "Read", "Grep", "mcp__tokred__tokred_retrieve"]);
    let text = (0usize..900, 1usize..90).prop_map(|(lines, width)| {
        (0..lines)
            .map(|n| format!("{n:0width$}\n"))
            .collect::<Vec<_>>()
            .concat()
    });
    let content = prop_oneof![
        text.clone().prop_map(Value::String),
        prop::collection::vec(text.clone(), 1..3).prop_map(|texts| json!(
            texts
                .into_iter()
                .map(|t| json!({"type": "text", "text": t}))
                .collect::<Vec<_>>()
        )),
        text.prop_map(|t| json!([{"type": "text", "text": t},
                                 {"type": "image", "source": {"type": "base64", "data": "AA"}}])),
    ];
    (
        tool.prop_map(str::to_owned),
        content,
        prop::bool::weighted(0.1),
    )
}

fn conversation(results: &[(String, Value, bool)]) -> Conversation {
    let mut conversation = Conversation::new("go");
    for (tool, content, is_error) in results {
        conversation.tool_turn(tool, content.clone());
        if *is_error {
            let last = conversation.messages.len() - 1;
            conversation.messages[last]["content"][0]["is_error"] = json!(true);
        }
    }
    conversation
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// In `observe` (and in `enforce` without the MCP server) the body sent upstream is the body
    /// received, byte for byte, on every turn.
    #[test]
    fn observe_never_changes_a_byte(results in prop::collection::vec(result(), 1..6)) {
        for (mode, registered) in [(TrafficMode::Observe, true), (TrafficMode::Enforce, false)] {
            let sink = Sink::new();
            let hook = hook(mode, registered, &sink);
            let cache = ResponseCacheShadow::new(true, std::sync::Arc::clone(&sink) as std::sync::Arc<dyn RecordSink>);
            for turn in 1..=results.len() {
                let body = conversation(&results[..turn]).body();
                let bytes = body.to_string();
                let sent = run(&cache, &meta(Some("s"), None), body, 1);
                let sent = run(&hook, &meta(Some("s"), None), sent.body, 1);
                prop_assert!(!sent.changed);
                prop_assert_eq!(sent.blocks, 0);
                prop_assert_eq!(sent.body.to_string(), bytes);
            }
            prop_assert!(sink.stored().is_empty());
        }
    }

    /// In `enforce`, every turn repeats the bytes of the previous turn, and two proxies given the
    /// same conversation send the same bytes.
    #[test]
    fn enforce_is_deterministic_and_prefix_stable(results in prop::collection::vec(result(), 1..6)) {
        let first = hook(TrafficMode::Enforce, true, &Sink::new());
        let second = hook(TrafficMode::Enforce, true, &Sink::new());
        let mut previous: Option<String> = None;
        for turn in 1..=results.len() {
            let body = conversation(&results[..turn]).body();
            let a = run(&first, &meta(Some("s"), None), body.clone(), 1);
            let b = run(&second, &meta(Some("s"), None), body, 1);
            prop_assert_eq!(&a, &b);
            let text = a.body.to_string();
            let open = text.strip_suffix("]}").unwrap().to_owned();
            if let Some(previous) = &previous {
                prop_assert!(open.starts_with(previous.as_str()));
            }
            previous = Some(open);
        }
    }
}
