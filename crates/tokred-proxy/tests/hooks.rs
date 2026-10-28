//! The hook pipeline end to end: rewrites, fail-open, the fallback resend and the cache-miss
//! explanation.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;

use hyper::Method;
use serde_json::Value;
use support::{
    Behavior, FakeUpstream, ManualClock, REWRITTEN, Reply, SECRET, hook, main_body, post_messages,
    send, sse_body, start, start_with,
};

const SUBAGENT: [(&str, &str); 2] = [
    ("x-claude-code-session-id", "s1"),
    ("x-claude-code-agent-id", "agent-1"),
];

fn ok_stream() -> Reply {
    Reply::stream(vec![sse_body("msg_ok", REWRITTEN)]).header("request-id", "req_ok")
}

fn model_of(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body).unwrap()["model"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn a_hook_that_rewrites_the_model_reaches_the_upstream() {
    let upstream = FakeUpstream::always(ok_stream()).await;
    let (rewrite, calls) = hook(Behavior::Rewrite);
    let proxy = start(&upstream.url(), vec![rewrite]).await;
    let original = main_body("claude-sonnet-5-5", &["Read", "Grep"]);

    let answer = post_messages(proxy.addr, &SUBAGENT, &original).await;
    assert_eq!(answer.status, 200);
    let seen = upstream.seen();
    assert_eq!(model_of(&seen[0].body), REWRITTEN);
    assert_eq!(
        seen[0].header("content-length"),
        Some(seen[0].body.len().to_string().as_str())
    );

    let record = &proxy.sink.wait_for(1).await[0];
    assert_eq!(record.origin, "subagent");
    assert_eq!(record.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(record.model_requested, "claude-sonnet-5-5");
    assert_eq!(record.model_effective, REWRITTEN);
    assert_eq!(record.notes.model_rewritten.as_deref(), Some(REWRITTEN));
    assert_eq!(record.notes.decision_id.as_deref(), Some("d1"));
    assert_eq!(record.bytes_in, original.len() as u64);
    assert_eq!(record.bytes_out, seen[0].body.len() as u64);
    assert_eq!(*calls.before.lock().unwrap(), ["subagent"]);
    assert!(calls.fallback.lock().unwrap().is_empty());
    proxy.stop().await;
}

#[tokio::test]
async fn the_main_loop_is_never_rewritten() {
    let upstream = FakeUpstream::always(ok_stream()).await;
    let (rewrite, _calls) = hook(Behavior::Rewrite);
    let proxy = start(&upstream.url(), vec![rewrite]).await;
    let original = main_body("claude-opus-5-5", &["Read"]);
    post_messages(proxy.addr, &[("x-claude-code-session-id", "s1")], &original).await;
    assert_eq!(&upstream.seen()[0].body[..], &original[..]);
    proxy.stop().await;
}

#[tokio::test]
async fn a_failing_or_panicking_hook_forwards_the_original_bytes() {
    for failing in [Behavior::Fail, Behavior::Panic] {
        let upstream = FakeUpstream::always(ok_stream()).await;
        let (rewrite, _) = hook(Behavior::Rewrite);
        let (bad, _) = hook(failing);
        let (after_bad, after_calls) = hook(Behavior::Observe);
        let proxy = start(&upstream.url(), vec![rewrite, bad, after_bad]).await;
        let original = main_body("claude-sonnet-5-5", &["Read"]);

        let answer = post_messages(proxy.addr, &SUBAGENT, &original).await;
        assert_eq!(answer.status, 200, "{failing:?}");
        assert_eq!(&upstream.seen()[0].body[..], &original[..], "{failing:?}");
        let record = &proxy.sink.wait_for(1).await[0];
        assert_eq!(record.notes.model_rewritten, None);
        assert_eq!(record.bytes_out, record.bytes_in);
        assert!(
            after_calls.before.lock().unwrap().is_empty(),
            "the pipeline stopped"
        );
        assert_eq!(
            after_calls.after.lock().unwrap().len(),
            1,
            "after still runs"
        );

        let status = send(
            proxy.addr,
            Method::GET,
            "/__tokred/status",
            &[("x-tokred-secret", SECRET)],
            "",
        )
        .await
        .json();
        assert_eq!(status["hook_failures"], 1);
        assert_eq!(status["messages"], 1);
        assert_eq!(status["hooks"].as_array().unwrap().len(), 3);
        proxy.stop().await;
    }
}

#[tokio::test]
async fn a_rejected_rewrite_is_resent_once_with_the_original_bytes() {
    for rejection in [400, 404, 422] {
        let upstream = FakeUpstream::start(Arc::new(move |seen| {
            if model_of(&seen.body) == REWRITTEN {
                Reply::json(
                    rejection,
                    r#"{"type":"error","error":{"type":"invalid_request_error","message":"model"}}"#,
                )
            } else {
                Reply::stream(vec![sse_body("msg_orig", "claude-sonnet-5-5")])
            }
        }))
        .await;
        let (rewrite, calls) = hook(Behavior::Rewrite);
        let proxy = start(&upstream.url(), vec![rewrite]).await;
        let original = main_body("claude-sonnet-5-5", &["Read"]);

        let answer = post_messages(proxy.addr, &SUBAGENT, &original).await;
        assert_eq!(answer.status, 200, "{rejection}");
        assert_eq!(
            &answer.body[..],
            &sse_body("msg_orig", "claude-sonnet-5-5")[..]
        );
        let seen = upstream.seen();
        assert_eq!(seen.len(), 2, "exactly one resend");
        assert_eq!(model_of(&seen[0].body), REWRITTEN);
        assert_eq!(&seen[1].body[..], &original[..]);
        assert_eq!(
            seen[1].header("authorization"),
            seen[0].header("authorization")
        );
        assert_eq!(
            *calls.fallback.lock().unwrap(),
            [(REWRITTEN.to_owned(), rejection)]
        );

        let record = &proxy.sink.wait_for(1).await[0];
        assert_eq!(record.status, 200);
        assert_eq!(record.bytes_out, original.len() as u64);
        assert_eq!(record.model_effective, "claude-sonnet-5-5");
        assert_eq!(calls.after.lock().unwrap()[0].status, 200);
        proxy.stop().await;
    }
}

#[tokio::test]
async fn only_changed_requests_rejected_with_400_404_or_422_are_resent() {
    let upstream = FakeUpstream::always(Reply::json(500, r#"{"type":"error"}"#)).await;
    let (rewrite, calls) = hook(Behavior::Rewrite);
    let proxy = start(&upstream.url(), vec![rewrite]).await;
    let answer = post_messages(proxy.addr, &SUBAGENT, &main_body("claude-sonnet-5-5", &[])).await;
    assert_eq!(answer.status, 500);
    assert_eq!(upstream.seen().len(), 1);
    assert!(calls.fallback.lock().unwrap().is_empty());
    proxy.stop().await;

    let upstream = FakeUpstream::always(Reply::json(400, r#"{"type":"error"}"#)).await;
    let (observe, _) = hook(Behavior::Observe);
    let proxy = start(&upstream.url(), vec![observe]).await;
    let answer = post_messages(proxy.addr, &SUBAGENT, &main_body("claude-sonnet-5-5", &[])).await;
    assert_eq!(answer.status, 400);
    assert_eq!(
        upstream.seen().len(),
        1,
        "nothing was changed, nothing to fall back to"
    );
    proxy.stop().await;
}

/// A main-loop response that read `cache_read` tokens and wrote `cache_write`.
fn cache_reply(cache_read: u64, cache_write: u64) -> Reply {
    let start = format!(
        r#"{{"type":"message_start","message":{{"id":"msg_c","model":"claude-opus-5-5","content":[],"usage":{{"input_tokens":3,"output_tokens":1,"cache_creation_input_tokens":{cache_write},"cache_read_input_tokens":{cache_read}}}}}}}"#
    );
    Reply::stream(vec![
        format!("event: message_start\ndata: {start}\n\n").into_bytes(),
        b"event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_vec(),
    ])
}

#[tokio::test]
async fn cache_misses_of_the_main_loop_are_explained() {
    let replies = Arc::new(std::sync::Mutex::new(vec![
        cache_reply(0, 40_000),
        cache_reply(0, 40_100),
        cache_reply(0, 40_200),
        cache_reply(0, 40_300),
    ]));
    let upstream = FakeUpstream::start(Arc::new(move |_| replies.lock().unwrap().remove(0))).await;
    let clock = Arc::new(ManualClock::default());
    let mut config = support::config(&upstream.url());
    config.clock = clock.clone();
    let proxy = start_with(config, vec![]).await;
    let session = [("x-claude-code-session-id", "sess")];

    clock.set(1_000);
    post_messages(
        proxy.addr,
        &session,
        &main_body("claude-opus-5-5", &["Read"]),
    )
    .await;
    clock.set(1_400);
    post_messages(
        proxy.addr,
        &session,
        &main_body("claude-opus-5-5", &["Read"]),
    )
    .await;
    clock.set(1_410);
    post_messages(
        proxy.addr,
        &session,
        &main_body("claude-opus-5-5", &["Read", "Mcp"]),
    )
    .await;
    clock.set(1_420);
    post_messages(
        proxy.addr,
        &SUBAGENT,
        &main_body("claude-opus-5-5", &["Read"]),
    )
    .await;

    let causes: Vec<Option<String>> = proxy
        .sink
        .wait_for(4)
        .await
        .into_iter()
        .map(|r| r.miss_cause)
        .collect();
    assert_eq!(
        causes,
        [
            None,
            Some("idle_ttl".to_owned()),
            Some("tools_changed".to_owned()),
            None
        ]
    );
    let records = proxy.sink.requests();
    assert_eq!(records[1].ts.0, 1_400);
    let status = send(
        proxy.addr,
        Method::GET,
        "/__tokred/status",
        &[("x-tokred-secret", SECRET)],
        "",
    )
    .await
    .json();
    assert_eq!(status["sessions"], 1);
    proxy.stop().await;
}
