//! Streaming: SSE split anywhere, JSON bodies, broken streams, no buffering, concurrency.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::time::{Duration, Instant};

use http_body_util::BodyExt;
use hyper::Method;
use support::{
    Behavior, FakeUpstream, Reply, hook, main_body, post_messages, request, split_at, sse_body,
    start,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn an_sse_stream_split_at_awkward_points_arrives_intact_and_is_measured() {
    let body = sse_body("msg_1", "claude-opus-5-5");
    // Cuts inside "event:", inside a JSON string, between "\n" and "\n", and one byte chunks.
    let text = String::from_utf8(body.clone()).unwrap();
    let in_event = text.find("vent: message_start").unwrap();
    let in_json = text.find("cache_read_input").unwrap() + 5;
    let between_newlines = text.find("\n\n").unwrap() + 1;
    let mut cuts = vec![in_event, between_newlines, in_json];
    let tail = text.find("message_delta").unwrap();
    cuts.extend((tail..tail + 40).step_by(1));
    cuts.sort_unstable();
    cuts.dedup();
    let upstream =
        FakeUpstream::always(Reply::stream(split_at(&body, &cuts)).header("request-id", "req_abc"))
            .await;
    let (observer, calls) = hook(Behavior::Observe);
    let proxy = start(&upstream.url(), vec![observer]).await;

    let answer = post_messages(
        proxy.addr,
        &[("x-claude-code-session-id", "s1")],
        &main_body("claude-opus-5-5", &["Read", "Grep"]),
    )
    .await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.header("content-type"), Some("text/event-stream"));
    assert_eq!(&answer.body[..], &body[..], "forwarded byte for byte");

    let record = &proxy.sink.wait_for(1).await[0];
    assert_eq!(record.message_key.as_deref(), Some("msg_1:req_abc"));
    assert_eq!(record.model_requested, "claude-opus-5-5");
    assert_eq!(record.model_effective, "claude-opus-5-5");
    assert_eq!(record.origin, "main");
    assert_eq!(
        (
            record.usage.input,
            record.usage.output,
            record.usage.cache_write,
            record.usage.cache_write_1h,
            record.usage.cache_read
        ),
        (12, 42, 300, 200, 4000)
    );
    assert_eq!(record.thinking_tokens, 17);
    assert_eq!(record.tools_hash.len(), 64);
    assert_eq!(record.system_hash.len(), 64);
    assert_ne!(record.tools_hash, record.system_hash);
    assert_eq!(record.miss_cause, None, "first request of the session");

    let summary = calls.after.lock().unwrap()[0].clone();
    assert_eq!(summary.tool_uses, ["Read"]);
    assert_eq!(summary.stop_reason.as_deref(), Some("tool_use"));
    assert_eq!(summary.request_id.as_deref(), Some("req_abc"));
    assert!(!summary.interrupted);
    assert!(summary.total_ms >= summary.ttfb_ms);
    proxy.stop().await;
}

#[tokio::test]
async fn a_json_response_is_measured() {
    let json = r#"{"id":"msg_j","type":"message","model":"claude-haiku-5-5","stop_reason":"end_turn","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":5,"output_tokens":6,"cache_read_input_tokens":7}}"#;
    let upstream = FakeUpstream::always(Reply::json(200, json).header("request-id", "req_j")).await;
    let proxy = start(&upstream.url(), vec![]).await;
    let answer = post_messages(
        proxy.addr,
        &[],
        br#"{"model":"claude-haiku-5-5","messages":[{"role":"user","content":"x"}]}"#,
    )
    .await;
    assert_eq!(&answer.body[..], json.as_bytes());
    assert_eq!(
        answer.header("content-length"),
        Some(json.len().to_string().as_str())
    );
    let record = &proxy.sink.wait_for(1).await[0];
    assert_eq!(record.message_key.as_deref(), Some("msg_j:req_j"));
    assert_eq!(record.usage.cache_read, 7);
    assert_eq!(record.status, 200);
    proxy.stop().await;
}

#[tokio::test]
async fn an_upstream_that_drops_mid_stream_is_recorded_as_interrupted() {
    let body = sse_body("msg_cut", "claude-opus-5-5");
    let first_event_end = body.windows(2).position(|w| w == b"\n\n").unwrap() + 2;
    let mut reply = Reply::stream(split_at(&body, &[first_event_end + 10]));
    reply.chunks.truncate(1);
    reply.abort = true;
    let upstream = FakeUpstream::always(reply).await;
    let (observer, calls) = hook(Behavior::Observe);
    let proxy = start(&upstream.url(), vec![observer]).await;

    let response = support::client()
        .request(request(
            proxy.addr,
            Method::POST,
            "/v1/messages",
            &[],
            main_body("claude-opus-5-5", &["Read"]),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(
        response.into_body().collect().await.is_err(),
        "the client sees the break"
    );

    let record = &proxy.sink.wait_for(1).await[0];
    assert_eq!(
        record.usage.cache_read, 4000,
        "message_start arrived before the cut"
    );
    assert!(calls.after.lock().unwrap()[0].interrupted);
    proxy.stop().await;
}

#[tokio::test]
async fn a_stream_that_ends_without_message_stop_is_interrupted() {
    let body = sse_body("msg_eof", "claude-opus-5-5");
    let upstream = FakeUpstream::always(Reply::stream(vec![body[..300].to_vec()])).await;
    let (observer, calls) = hook(Behavior::Observe);
    let proxy = start(&upstream.url(), vec![observer]).await;
    let answer = post_messages(proxy.addr, &[], &main_body("claude-opus-5-5", &["Read"])).await;
    assert_eq!(&answer.body[..], &body[..300]);
    proxy.sink.wait_for(1).await;
    assert!(calls.after.lock().unwrap()[0].interrupted);
    proxy.stop().await;
}

#[tokio::test]
async fn a_client_that_leaves_mid_stream_closes_the_exchange() {
    let events: Vec<Vec<u8>> = support::sse_events("msg_gone", "claude-opus-5-5")
        .into_iter()
        .map(String::into_bytes)
        .collect();
    let mut reply = Reply::stream(events);
    reply.gap = Duration::from_millis(100);
    let upstream = FakeUpstream::always(reply).await;
    let (observer, calls) = hook(Behavior::Observe);
    let proxy = start(&upstream.url(), vec![observer]).await;

    let body = main_body("claude-opus-5-5", &["Read"]);
    let mut stream = tokio::net::TcpStream::connect(proxy.addr).await.unwrap();
    let head = format!(
        "POST /v1/messages HTTP/1.1\r\nhost: 127.0.0.1:{}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
        proxy.addr.port(),
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(&body).await.unwrap();
    let mut first = [0_u8; 64];
    let read = stream.read(&mut first).await.unwrap();
    assert!(String::from_utf8_lossy(&first[..read]).starts_with("HTTP/1.1 200"));
    drop(stream);

    let record = &proxy.sink.wait_for(1).await[0];
    assert_eq!(record.status, 200);
    assert!(calls.after.lock().unwrap()[0].interrupted);
    proxy.stop().await;
}

#[tokio::test]
async fn the_response_is_streamed_not_buffered() {
    let events: Vec<Vec<u8>> = support::sse_events("msg_slow", "claude-opus-5-5")
        .into_iter()
        .map(String::into_bytes)
        .collect();
    let mut reply = Reply::stream(events);
    reply.gap = Duration::from_millis(150);
    let upstream = FakeUpstream::always(reply).await;
    let proxy = start(&upstream.url(), vec![]).await;

    let started = Instant::now();
    let response = support::client()
        .request(request(
            proxy.addr,
            Method::POST,
            "/v1/messages",
            &[],
            main_body("claude-opus-5-5", &["Read"]),
        ))
        .await
        .unwrap();
    let mut body = response.into_body();
    let first = body.frame().await.unwrap().unwrap();
    let first_at = started.elapsed();
    assert!(
        first
            .data_ref()
            .unwrap()
            .starts_with(b"event: message_start"),
        "the first event arrives on its own"
    );
    let rest = body.collect().await.unwrap();
    let total = started.elapsed();
    assert!(!rest.to_bytes().is_empty());
    assert!(
        first_at + Duration::from_millis(600) < total,
        "first event after {first_at:?}, end after {total:?}"
    );
    proxy.stop().await;
}

#[tokio::test]
async fn compressed_responses_are_forwarded_but_not_read() {
    let gzip_bytes = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3, 1, 2, 3];
    let mut reply = Reply::stream(vec![gzip_bytes.clone()]);
    reply
        .headers
        .push(("content-encoding".into(), "gzip".into()));
    let upstream = FakeUpstream::always(reply).await;
    let proxy = start(&upstream.url(), vec![]).await;
    let answer = post_messages(
        proxy.addr,
        &[("accept-encoding", "gzip")],
        &main_body("claude-opus-5-5", &["Read"]),
    )
    .await;
    assert_eq!(&answer.body[..], &gzip_bytes[..]);
    assert_eq!(answer.header("content-encoding"), Some("gzip"));
    assert_eq!(upstream.seen()[0].header("accept-encoding"), Some("gzip"));
    let record = &proxy.sink.wait_for(1).await[0];
    assert_eq!(record.usage.total(), 0);
    assert_eq!(record.status, 200);
    proxy.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fifty_streams_at_once() {
    let body = sse_body("msg_many", "claude-sonnet-5-5");
    let chunks = split_at(&body, &[50, 400, 401, 900]);
    let upstream = FakeUpstream::always(Reply::stream(chunks).header("request-id", "req_m")).await;
    let proxy = start(&upstream.url(), vec![]).await;
    let mut tasks = Vec::new();
    for index in 0..50 {
        let addr = proxy.addr;
        let session = format!("s{index}");
        tasks.push(tokio::spawn(async move {
            post_messages(
                addr,
                &[("x-claude-code-session-id", session.as_str())],
                &main_body("claude-sonnet-5-5", &["Read"]),
            )
            .await
        }));
    }
    for task in tasks {
        let answer = task.await.unwrap();
        assert_eq!(answer.status, 200);
        assert_eq!(&answer.body[..], &body[..]);
    }
    let records = proxy.sink.wait_for(50).await;
    assert_eq!(records.len(), 50);
    assert!(
        records
            .iter()
            .all(|r| r.usage.output == 42 && r.status == 200)
    );
    let ids: std::collections::HashSet<&str> = records.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids.len(), 50);
    proxy.stop().await;
}
