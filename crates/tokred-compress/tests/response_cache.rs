//! The response cache shadow hook.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;

use common::{Sink, meta, run};
use serde_json::json;
use tokred_compress::response_cache::REPEAT_WINDOW_SECS;
use tokred_compress::{ResponseCacheShadow, canonical_hash};
use tokred_core::Timestamp;
use tokred_core::proxy::{ProxyHook, RecordSink};

#[test]
fn exact_repeats_within_a_day_are_recorded_and_nothing_changes() {
    let sink = Sink::new();
    let hook = ResponseCacheShadow::new(true, Arc::clone(&sink) as Arc<dyn RecordSink>);
    assert_eq!(hook.name(), "response-cache-shadow");
    assert!(format!("{hook:?}").contains("enabled: true"));

    let body =
        json!({"model": "claude-haiku-5-5", "messages": [{"role": "user", "content": "title"}]});
    let reordered =
        json!({"messages": [{"content": "title", "role": "user"}], "model": "claude-haiku-5-5"});
    let meta = meta(Some("s1"), None);

    let first = run(&hook, &meta, body.clone(), 100);
    assert_eq!((first.body, first.changed), (body.clone(), false));
    assert!(sink.shadow().is_empty(), "a first sighting is not a repeat");

    let again = run(&hook, &meta, reordered.clone(), 200);
    assert_eq!(again.body, reordered, "the body is never altered");
    run(&hook, &meta, body.clone(), 200 + REPEAT_WINDOW_SECS + 1);

    let records = sink.shadow();
    assert_eq!(records.len(), 1);
    let (hash, bytes) = canonical_hash(&body);
    assert_eq!(records[0].ts, Timestamp(200));
    assert_eq!(records[0].session_id, "s1");
    assert_eq!(records[0].kind, "response_cache");
    assert_eq!(records[0].key, hash);
    assert_eq!(records[0].bytes_saved, bytes as u64);
    assert_eq!(records[0].est_tokens_saved, (bytes / 4) as u64);
    assert_eq!(records[0].rules_version, "r1");

    // Without a session id the record still counts, with an empty session.
    run(&hook, &meta_none(), body, 200 + REPEAT_WINDOW_SECS + 2);
    assert_eq!(sink.shadow()[1].session_id, "");
}

fn meta_none() -> tokred_core::proxy::RequestMeta {
    meta(None, None)
}

#[test]
fn disabled_it_records_nothing() {
    let sink = Sink::new();
    let hook = ResponseCacheShadow::new(false, Arc::clone(&sink) as Arc<dyn RecordSink>);
    let body = json!({"messages": []});
    for now in 0..3 {
        assert_eq!(
            run(&hook, &meta(Some("s"), None), body.clone(), now).body,
            body
        );
    }
    assert!(sink.all().is_empty());
}
