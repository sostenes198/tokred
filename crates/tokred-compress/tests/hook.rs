//! The compression hook against simulated Claude Code sessions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::{Arc, Mutex};

use common::{Conversation, Sink, big, hook, meta, result_text, run};
use serde_json::{Value, json};
use tokred_compress::preview::{PreviewRules, content_hash, replacement, retrieve_id};
use tokred_compress::{CompressionHook, conversation_key};
use tokred_core::Timestamp;
use tokred_core::config::{CompressionConfig, ShadowConfig, TrafficMode};
use tokred_core::proxy::{ProxyHook, RecordSink};

const MARKER: &str = "[tokred: ";

/// The serialized body without its closing `]}`: with `messages` last, the bytes of one turn
/// must be a prefix of the next turn's.
fn open_prefix(body: &Value) -> String {
    let text = body.to_string();
    text.strip_suffix("]}").unwrap().to_owned()
}

#[test]
fn a_six_turn_session_sends_the_same_bytes_on_every_turn() {
    let sink = Sink::new();
    let hook = hook(TrafficMode::Enforce, true, &sink);
    let meta = meta(Some("s1"), None);
    let mut conversation = Conversation::new("investigate the build");
    let mut previous: Option<Value> = None;

    for turn in 1..=6 {
        conversation.tool_turn("Bash", json!(big(&format!("turn{turn}"), 400)));
        let sent = run(&hook, &meta, conversation.body(), 1000 + turn);
        assert!(sent.changed);
        assert_eq!(
            sent.blocks,
            u32::try_from(turn).unwrap(),
            "every replaced block, every turn"
        );

        // The newest result is replaced the first time it is sent.
        let newest = result_text(&sent.body, conversation.messages.len() - 1);
        assert!(newest.contains(MARKER), "turn {turn}");
        if let Some(previous) = &previous {
            assert!(
                open_prefix(&sent.body).starts_with(&open_prefix(previous)),
                "turn {turn} changed bytes the model already saw"
            );
        }
        // Nothing outside the tool results changed.
        let original = conversation.body();
        for key in [
            "model",
            "system",
            "tools",
            "thinking",
            "max_tokens",
            "stream",
        ] {
            assert_eq!(sent.body[key], original[key]);
        }
        previous = Some(sent.body);
    }

    let stored = sink.stored();
    let compressed = sink.compressed();
    assert_eq!((stored.len(), compressed.len()), (6, 6), "stored once each");
    for (index, (stored, block)) in stored.iter().zip(&compressed).enumerate() {
        let turn = index + 1;
        let original = big(&format!("turn{turn}"), 400);
        assert_eq!(stored.content, original, "nothing is lost");
        assert_eq!(stored.original_hash, content_hash(&original));
        assert_eq!(stored.id, retrieve_id(&stored.original_hash));
        assert_eq!(
            (stored.tool.as_str(), stored.session_id.as_str()),
            ("Bash", "s1")
        );
        assert_eq!((stored.lines, stored.bytes), (400, original.len() as u64));
        assert_eq!(stored.expires_at.0 - stored.created_at.0, 7 * 86_400);
        assert_eq!(block.session_id, "s1");
        assert_eq!(block.original_hash, stored.original_hash);
        assert_eq!(block.rules_version, "r1-h40-t20");
        assert_eq!(
            Some(block.replacement.clone()),
            replacement(&original, &PreviewRules::default()).map(|r| r.text)
        );
    }
    assert!(
        sink.shadow().is_empty(),
        "enforce records real replacements, not shadow ones"
    );
}

#[test]
fn a_restarted_proxy_repeats_the_same_bytes_from_the_store() {
    let sink = Sink::new();
    let first = hook(TrafficMode::Enforce, true, &sink);
    let meta = meta(Some("s1"), None);
    let mut conversation = Conversation::new("go");
    for turn in 1..=3 {
        conversation.tool_turn("Bash", json!(big(&format!("t{turn}"), 300)));
        run(&first, &meta, conversation.body(), turn);
    }
    conversation.tool_turn("Bash", json!(big("t4", 300)));
    let before_restart = run(&first, &meta, conversation.body(), 4);

    // Seeded at startup.
    let seeded = hook(TrafficMode::Enforce, true, &Sink::new());
    seeded.seed(sink.compressed());
    assert_eq!(run(&seeded, &meta, conversation.body(), 5), before_restart);

    // Or loaded the first time the conversation shows up.
    let records = sink.compressed();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&calls);
    let loading =
        hook(TrafficMode::Enforce, true, &Sink::new()).with_loader(Box::new(move |key: &str| {
            seen.lock().unwrap().push(key.to_owned());
            records
                .iter()
                .filter(|b| b.session_id == key)
                .cloned()
                .collect()
        }));
    assert_eq!(run(&loading, &meta, conversation.body(), 5), before_restart);
    assert_eq!(run(&loading, &meta, conversation.body(), 6), before_restart);
    assert_eq!(*calls.lock().unwrap(), ["s1"], "loaded once");
}

#[test]
fn the_map_wins_over_a_fresh_computation() {
    let meta = meta(Some("s1"), None);
    let mut conversation = Conversation::new("go");
    conversation.tool_turn("Bash", json!(big("x", 300)));
    let original = big("x", 300);
    // Replacement bytes written by an earlier version of the rules.
    let seeded = hook(TrafficMode::Enforce, true, &Sink::new());
    seeded.seed([tokred_core::compression::CompressedBlock {
        session_id: "s1".to_owned(),
        original_hash: content_hash(&original),
        replacement: "older preview".to_owned(),
        rules_version: "r0".to_owned(),
        ts: Timestamp(1),
    }]);
    // And a second seed for the same block does not overwrite the first.
    seeded.seed([tokred_core::compression::CompressedBlock {
        session_id: "s1".to_owned(),
        original_hash: content_hash(&original),
        replacement: "newer preview".to_owned(),
        rules_version: "r9".to_owned(),
        ts: Timestamp(2),
    }]);
    let sent = run(&seeded, &meta, conversation.body(), 3);
    assert_eq!(result_text(&sent.body, 2), "older preview");
}

#[test]
fn claude_code_moving_the_breakpoint_keeps_the_replaced_text_identical() {
    let sink = Sink::new();
    let hook = hook(TrafficMode::Enforce, true, &sink);
    let meta = meta(Some("s1"), None);
    let mut conversation = Conversation::new("go");
    conversation.move_breakpoint = true;
    let mut texts: Vec<String> = Vec::new();
    for turn in 1..=4 {
        conversation.tool_turn("Bash", json!(big(&format!("b{turn}"), 300)));
        let sent = run(&hook, &meta, conversation.body(), turn);
        let last = conversation.messages.len() - 1;
        // The breakpoint stays where Claude Code put it, next to the replaced text.
        assert_eq!(
            sent.body["messages"][last]["content"][0]["cache_control"],
            json!({"type": "ephemeral"})
        );
        for (index, text) in texts.iter().enumerate() {
            assert_eq!(result_text(&sent.body, 2 * index + 2), text);
        }
        texts.push(result_text(&sent.body, last).to_owned());
    }
    assert!(texts.iter().all(|t| t.contains(MARKER)));
}

#[test]
fn only_new_blocks_in_the_last_message_are_replaced() {
    let sink = Sink::new();
    let hook = hook(TrafficMode::Enforce, true, &sink);
    let meta = meta(Some("s1"), None);
    let mut conversation = Conversation::new("go");
    // Seen by the model before tokred was in enforce (not in the map).
    conversation.tool_turn("Bash", json!(big("old", 300)));
    conversation.tool_turn("Bash", json!(big("new", 300)));
    let sent = run(&hook, &meta, conversation.body(), 1);
    assert_eq!(
        result_text(&sent.body, 2),
        big("old", 300),
        "seen whole stays whole"
    );
    assert!(result_text(&sent.body, 4).contains(MARKER));
    assert_eq!(sent.blocks, 1);

    // A later turn keeps both as they were.
    conversation.tool_turn("Bash", json!("small output"));
    let next = run(&hook, &meta, conversation.body(), 2);
    assert_eq!(result_text(&next.body, 2), big("old", 300));
    assert_eq!(result_text(&next.body, 4), result_text(&sent.body, 4));
    assert_eq!(result_text(&next.body, 6), "small output");

    // A user message that is not the last one never gets new replacements, and neither does
    // a request whose last message is the assistant's.
    let mut prefilled = Conversation::new("go");
    prefilled.tool_turn("Bash", json!(big("pre", 300)));
    prefilled
        .messages
        .push(json!({"role": "assistant", "content": "partial"}));
    let untouched = run(&hook, &meta, prefilled.body(), 3);
    assert!(!untouched.changed);
    assert_eq!(untouched.body, prefilled.body());
}

#[test]
fn text_the_model_saw_whole_earlier_is_never_replaced_later() {
    let sink = Sink::new();
    let hook = hook(TrafficMode::Enforce, true, &sink);
    let meta = meta(Some("s1"), None);
    let mut conversation = Conversation::new("go");
    conversation.tool_turn("Bash", json!(big("same", 300)));
    // The same output again: replacing it now would make the older copy match the map next turn.
    conversation.tool_turn("Bash", json!(big("same", 300)));
    let sent = run(&hook, &meta, conversation.body(), 1);
    assert!(!sent.changed);
    assert_eq!(sent.body, conversation.body());
    assert!(sink.compressed().is_empty());

    conversation.tool_turn("Bash", json!(big("same", 300)));
    let next = run(&hook, &meta, conversation.body(), 2);
    assert_eq!(next.body, conversation.body());
}

#[test]
fn identical_results_in_one_message_share_one_replacement() {
    let sink = Sink::new();
    let hook = hook(TrafficMode::Enforce, true, &sink);
    let mut conversation = Conversation::new("go");
    conversation.tool_turn("Bash", json!(big("twin", 300)));
    let last = conversation.messages.len() - 1;
    let twin = conversation.messages[last]["content"][0].clone();
    conversation.messages[last]["content"]
        .as_array_mut()
        .unwrap()
        .push(twin);
    let sent = run(&hook, &meta(Some("s1"), None), conversation.body(), 1);
    let blocks = sent.body["messages"][last]["content"].as_array().unwrap();
    assert_eq!(blocks[0], blocks[1]);
    assert_eq!(sent.blocks, 2);
    assert_eq!((sink.stored().len(), sink.compressed().len()), (1, 1));
}

#[test]
fn a_subagent_never_changes_what_the_main_loop_saw() {
    let sink = Sink::new();
    let hook = hook(TrafficMode::Enforce, true, &sink);
    let main_meta = meta(Some("s1"), None);
    let agent_meta = meta(Some("s1"), Some("agent-7"));

    // The main loop saw this output whole.
    let mut main = Conversation::new("go");
    main.tool_turn("Bash", json!(big("shared", 300)));
    main.tool_turn("Bash", json!("ok"));
    run(&hook, &main_meta, main.body(), 1);

    // A subagent gets the same output as a new result: it is replaced there...
    let mut agent = Conversation::new("explore");
    agent.tool_turn("Bash", json!(big("shared", 300)));
    let sent = run(&hook, &agent_meta, agent.body(), 2);
    assert!(result_text(&sent.body, 2).contains(MARKER));
    assert_eq!(sink.compressed()[0].session_id, "s1/agent-7");
    assert_eq!(sink.stored()[0].session_id, "s1");

    // ...and the main loop's copy stays as the main loop saw it.
    main.tool_turn("Bash", json!("next"));
    let main_sent = run(&hook, &main_meta, main.body(), 3);
    assert_eq!(main_sent.body, main.body());
}

#[test]
fn images_documents_errors_and_retrievals_are_never_touched() {
    let sink = Sink::new();
    let hook = hook(TrafficMode::Enforce, true, &sink);
    let meta = meta(Some("s1"), None);
    let image = json!({"type": "image", "source": {"type": "base64", "media_type": "image/png",
                                                   "data": "iVBORw0KGgo".repeat(2000)}});
    let document = json!({"type": "document", "source": {"type": "text",
                          "media_type": "text/plain", "data": big("doc", 300)}});
    for content in [
        json!([{"type": "text", "text": big("with image", 300)}, image.clone()]),
        json!([image]),
        json!([document]),
    ] {
        let mut conversation = Conversation::new("go");
        conversation.tool_turn("Read", content);
        let sent = run(&hook, &meta, conversation.body(), 1);
        assert!(!sent.changed);
        assert_eq!(sent.body, conversation.body());
    }

    let mut conversation = Conversation::new("go");
    conversation.tool_turn("Bash", json!(big("err", 300)));
    let last = conversation.messages.len() - 1;
    conversation.messages[last]["content"][0]["is_error"] = json!(true);
    assert_eq!(
        run(&hook, &meta, conversation.body(), 1).body,
        conversation.body()
    );

    let mut conversation = Conversation::new("go");
    conversation.retrieve_turn("0123456789abcdef", &big("retrieved", 300));
    assert_eq!(
        run(&hook, &meta, conversation.body(), 1).body,
        conversation.body()
    );
    assert!(sink.all().is_empty());
}

#[test]
fn text_blocks_are_replaced_one_by_one_keeping_their_keys() {
    let sink = Sink::new();
    let hook = hook(TrafficMode::Enforce, true, &sink);
    let mut conversation = Conversation::new("go");
    conversation.tool_turn(
        "mcp__github__get_file",
        json!([
            {"type": "text", "text": big("part one", 300)},
            {"type": "text", "text": "short note", "cache_control": {"type": "ephemeral"}},
            {"type": "text", "text": big("part two", 300), "citations": []}
        ]),
    );
    let sent = run(&hook, &meta(Some("s1"), None), conversation.body(), 1);
    let items = sent.body["messages"][2]["content"][0]["content"]
        .as_array()
        .unwrap();
    assert!(items[0]["text"].as_str().unwrap().contains(MARKER));
    assert_eq!(items[1]["text"], "short note");
    assert_eq!(items[1]["cache_control"], json!({"type": "ephemeral"}));
    assert!(items[2]["text"].as_str().unwrap().contains(MARKER));
    assert_eq!(items[2]["citations"], json!([]));
    assert_eq!(sent.blocks, 2);
    assert!(
        sink.stored()
            .iter()
            .all(|s| s.tool == "mcp__github__get_file")
    );
}

#[test]
fn a_tool_retrieved_too_often_is_excluded_from_then_on() {
    let sink = Sink::new();
    let hook = hook(TrafficMode::Enforce, true, &sink);
    let meta = meta(Some("s1"), None);
    let mut conversation = Conversation::new("go");
    conversation.tool_turn("Bash", json!(big("first", 300)));
    run(&hook, &meta, conversation.body(), 1);
    conversation.tool_turn("Grep", json!(big("grep", 300)));
    run(&hook, &meta, conversation.body(), 1);
    let first_id = sink.stored()[0].id.clone();
    assert_eq!(sink.stored()[0].tool, "Bash");

    // Three retrievals: still within `max_retrievals`.
    for _ in 0..3 {
        conversation.retrieve_turn(&first_id, "page");
    }
    conversation.tool_turn("Bash", json!(big("second", 300)));
    let sent = run(&hook, &meta, conversation.body(), 2);
    assert!(result_text(&sent.body, conversation.messages.len() - 1).contains(MARKER));

    // A fourth: Bash results are now sent whole; what was replaced stays replaced.
    conversation.retrieve_turn(&first_id, "page");
    conversation.tool_turn("Bash", json!(big("third", 300)));
    conversation.tool_turn("Grep", json!(big("grep two", 300)));
    let last = conversation.messages.len() - 1;
    let sent = run(&hook, &meta, conversation.body(), 3);
    assert!(result_text(&sent.body, 2).contains(MARKER));
    assert!(
        result_text(&sent.body, last).contains(MARKER),
        "other tools are not affected"
    );
    conversation.tool_turn("Bash", json!(big("fourth", 300)));
    let sent = run(&hook, &meta, conversation.body(), 4);
    assert_eq!(
        result_text(&sent.body, conversation.messages.len() - 1),
        big("fourth", 300)
    );

    // The exclusion belongs to that conversation only.
    let mut other = Conversation::new("go");
    other.tool_turn("Bash", json!(big("elsewhere", 300)));
    let sent = run(&hook, &meta_for("s2"), other.body(), 5);
    assert!(result_text(&sent.body, 2).contains(MARKER));
}

fn meta_for(session: &str) -> tokred_core::proxy::RequestMeta {
    meta(Some(session), None)
}

#[test]
fn tools_excluded_by_the_cli_are_never_replaced() {
    let sink = Sink::new();
    let hook = hook(TrafficMode::Enforce, true, &sink);
    hook.exclude_tools(["Read".to_owned()]);
    let mut conversation = Conversation::new("go");
    conversation.tool_turn("Read", json!(big("file", 300)));
    let sent = run(&hook, &meta_for("s1"), conversation.body(), 1);
    assert!(!sent.changed);
}

#[test]
fn enforce_without_the_mcp_server_only_observes() {
    let sink = Sink::new();
    let hook = hook(TrafficMode::Enforce, false, &sink);
    assert_eq!(hook.effective_mode(), TrafficMode::Observe);
    let mut conversation = Conversation::new("go");
    conversation.tool_turn("Bash", json!(big("x", 300)));
    let sent = run(&hook, &meta_for("s1"), conversation.body(), 1);
    assert!(!sent.changed);
    assert_eq!(sent.body, conversation.body());
    assert!(sink.stored().is_empty() && sink.compressed().is_empty());
    assert_eq!(sink.shadow().len(), 1);
}

#[test]
fn observe_records_each_block_once_and_changes_nothing() {
    let sink = Sink::new();
    let hook = hook(TrafficMode::Observe, true, &sink);
    let meta = meta_for("s1");
    let mut conversation = Conversation::new("go");
    // Seeds are ignored in observe: nothing is ever replaced.
    hook.seed([tokred_core::compression::CompressedBlock {
        session_id: "s1".to_owned(),
        original_hash: content_hash(&big("t1", 300)),
        replacement: "would be wrong to send".to_owned(),
        ..Default::default()
    }]);
    for turn in 1..=4 {
        conversation.tool_turn("Bash", json!(big(&format!("t{turn}"), 300)));
        let sent = run(&hook, &meta, conversation.body(), 100 + turn);
        assert!(!sent.changed);
        assert_eq!(sent.blocks, 0);
        assert_eq!(sent.body, conversation.body());
    }
    let shadow = sink.shadow();
    assert_eq!(shadow.len(), 4, "once per block, not every turn");
    let original = big("t1", 300);
    let r = replacement(&original, &PreviewRules::default()).unwrap();
    assert_eq!(shadow[0].ts, Timestamp(101));
    assert_eq!(shadow[0].session_id, "s1");
    assert_eq!(shadow[0].kind, "compression");
    assert_eq!(shadow[0].key, "Bash");
    assert_eq!(
        shadow[0].bytes_saved,
        (original.len() - r.text.len()) as u64
    );
    assert_eq!(shadow[0].est_tokens_saved, shadow[0].bytes_saved / 4);
    assert_eq!(shadow[0].rules_version, "r1-h40-t20");
    assert!(sink.stored().is_empty());

    // Observe applies the same "seen whole" rule as enforce.
    let mut repeated = Conversation::new("go");
    repeated.tool_turn("Bash", json!(big("again", 300)));
    repeated.tool_turn("Bash", json!(big("again", 300)));
    run(&hook, &meta_for("s3"), repeated.body(), 1);
    assert_eq!(sink.shadow().len(), 4);
}

#[test]
fn off_no_shadow_and_no_session_do_nothing() {
    let mut conversation = Conversation::new("go");
    conversation.tool_turn("Bash", json!(big("x", 300)));

    let sink = Sink::new();
    let off = hook(TrafficMode::Off, true, &sink);
    assert_eq!(
        run(&off, &meta_for("s1"), conversation.body(), 1).body,
        conversation.body()
    );

    let quiet = CompressionHook::new(
        &common::config(TrafficMode::Observe),
        &ShadowConfig {
            compression: false,
            response_cache: true,
        },
        true,
        Arc::clone(&sink) as Arc<dyn RecordSink>,
    );
    run(&quiet, &meta_for("s1"), conversation.body(), 1);

    let enforce = hook(TrafficMode::Enforce, true, &sink);
    for no_session in [meta(None, None), meta(Some(""), Some("a"))] {
        let sent = run(&enforce, &no_session, conversation.body(), 1);
        assert!(!sent.changed);
    }
    // No tool results at all.
    let typed = run(
        &enforce,
        &meta_for("s1"),
        json!({"messages": [{"role": "user", "content": "hi"}]}),
        1,
    );
    assert!(!typed.changed);
    assert!(sink.all().is_empty());
}

#[test]
fn conversation_keys_separate_subagents() {
    assert_eq!(
        conversation_key(&meta(Some("s"), None)).as_deref(),
        Some("s")
    );
    assert_eq!(
        conversation_key(&meta(Some("s"), Some("a"))).as_deref(),
        Some("s/a")
    );
    assert_eq!(conversation_key(&meta(None, Some("a"))), None);
}

#[test]
fn an_unreadable_retention_falls_back_to_seven_days_and_the_hook_has_a_name() {
    let sink = Sink::new();
    let hook = CompressionHook::new(
        &CompressionConfig {
            mode: TrafficMode::Enforce,
            retention: "soon".to_owned(),
            ..CompressionConfig::default()
        },
        &ShadowConfig::default(),
        true,
        Arc::clone(&sink) as Arc<dyn RecordSink>,
    );
    assert_eq!(hook.name(), "compression");
    assert!(format!("{hook:?}").contains("Enforce"));
    let mut conversation = Conversation::new("go");
    conversation.tool_turn("Bash", json!(big("x", 300)));
    run(&hook, &meta_for("s1"), conversation.body(), 10);
    assert_eq!(sink.stored()[0].expires_at, Timestamp(10 + 7 * 86_400));
}

#[test]
fn a_rejected_request_keeps_its_conversation_whole_and_trips_the_breaker() {
    use tokred_core::proxy::{HookNotes, RequestCtx, RequestShape, classify_origin};

    let sink = Sink::new();
    let hook = hook(TrafficMode::Enforce, true, &sink);
    let mut other = Conversation::new("go");
    other.tool_turn("Bash", json!(big("other", 300)));
    let other_first = run(&hook, &meta_for("s2"), other.body(), 1);
    assert!(other_first.changed);

    let mut conversation = Conversation::new("go");
    conversation.tool_turn("Bash", json!(big("rejected", 300)));
    let meta = meta_for("s1");
    let body = conversation.body();
    let shape = RequestShape::of(&body);
    let origin = classify_origin(&meta, &shape);
    let mut ctx = RequestCtx {
        meta: &meta,
        shape: &shape,
        origin: &origin,
        now: Timestamp(2),
        body,
        changed: false,
        notes: HookNotes::default(),
    };
    hook.before(&mut ctx).unwrap();
    assert!(ctx.changed && !hook.is_tripped());
    hook.fallback(&ctx, 400);
    assert!(hook.is_tripped());

    // The rejected conversation now gets its originals on every turn.
    conversation.tool_turn("Bash", json!(big("after", 300)));
    assert_eq!(
        run(&hook, &meta, conversation.body(), 3).body,
        conversation.body()
    );

    // Other conversations keep the bytes already sent, but get no new replacements.
    other.tool_turn("Bash", json!(big("other new", 300)));
    let sent = run(&hook, &meta_for("s2"), other.body(), 4);
    assert_eq!(
        result_text(&sent.body, 2),
        result_text(&other_first.body, 2)
    );
    assert_eq!(result_text(&sent.body, 4), big("other new", 300));
}

#[test]
fn a_fallback_without_compression_changes_nothing() {
    use tokred_core::proxy::{HookNotes, Origin, RequestCtx, RequestShape};

    let sink = Sink::new();
    let hook = hook(TrafficMode::Enforce, true, &sink);
    let shape = RequestShape::default();
    for (meta, blocks) in [(meta_for("s1"), 0), (meta(None, None), 3)] {
        let ctx = RequestCtx {
            meta: &meta,
            shape: &shape,
            origin: &Origin::Main,
            now: Timestamp(1),
            body: json!({}),
            changed: true,
            notes: HookNotes {
                blocks_compressed: blocks,
                ..HookNotes::default()
            },
        };
        hook.fallback(&ctx, 400);
    }
    assert!(!hook.is_tripped());
}
