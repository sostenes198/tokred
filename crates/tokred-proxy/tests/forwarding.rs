//! Pass-through: bytes, headers, other endpoints, upstream errors and failures.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use hyper::Method;
use proptest::prelude::*;
use support::{
    Behavior, FakeUpstream, Reply, closed_port, hook, post_messages, raw, send, start, start_with,
};

fn ok_message() -> Reply {
    Reply::json(
        200,
        r#"{"id":"msg_x","type":"message","model":"claude-sonnet-5-5","content":[],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":2}}"#,
    )
    .header("request-id", "req_x")
}

/// JSON text whose bytes differ from a re-serialization: odd spacing, escapes, number forms.
fn json_text() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        Just("1.50".to_owned()),
        Just("1e2".to_owned()),
        Just("-0".to_owned()),
        Just("true".to_owned()),
        Just("null".to_owned()),
        Just(r#""caf\u00e9 \"q\" \/""#.to_owned()),
        "[a-z ]{0,8}".prop_map(|s| format!("\"{s}\"")),
        any::<i64>().prop_map(|n| n.to_string()),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4)
                .prop_map(|items| format!("[ {} ]", items.join(" ,\n\t"))),
            prop::collection::vec(("[a-z_]{1,6}", inner), 0..4).prop_map(|pairs| {
                let fields: Vec<String> =
                    pairs.iter().map(|(k, v)| format!("\"{k}\" :{v}")).collect();
                format!("{{ {} }}", fields.join(","))
            }),
        ]
    })
}

fn message_body() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        3 => (json_text(), any::<bool>()).prop_map(|(extra, tools)| {
            let tools = if tools { r#", "tools" : [ {"name":"Read"} ]"# } else { "" };
            format!(
                "{{ \"model\":\"claude-sonnet-5-5\" , \"extra\": {extra}{tools},\n \"messages\":[{{\"role\":\"user\",\"content\":\"hi\"}}] }}"
            )
            .into_bytes()
        }),
        1 => prop::collection::vec(any::<u8>(), 0..200),
    ]
}

#[test]
fn bodies_pass_byte_for_byte_when_no_hook_changes_them() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (upstream, proxy) = runtime.block_on(async {
        let upstream = FakeUpstream::always(ok_message()).await;
        let (observer, _calls) = hook(Behavior::Observe);
        let proxy = start(&upstream.url(), vec![observer]).await;
        (upstream, proxy)
    });
    proptest!(ProptestConfig::with_cases(64), |(body in message_body())| {
        let before = upstream.seen().len();
        let answer = runtime.block_on(post_messages(proxy.addr, &[], &body));
        prop_assert_eq!(answer.status, 200);
        let seen = upstream.seen();
        prop_assert_eq!(seen.len(), before + 1);
        prop_assert_eq!(&seen[before].body[..], &body[..]);
    });
    let sent = upstream.seen().len();
    let records = runtime.block_on(proxy.sink.wait_for(sent));
    assert!(records.iter().all(|r| r.bytes_in == r.bytes_out));
    runtime.block_on(proxy.stop());
}

#[tokio::test]
async fn headers_pass_unchanged_except_hop_by_hop_ones() {
    let upstream = FakeUpstream::always(ok_message()).await;
    let proxy = start(&upstream.url(), vec![]).await;
    let body = br#"{"model":"claude-opus-5-5","messages":[{"role":"user","content":"x"}]}"#;
    let headers = [
        ("authorization", "Bearer oauth-token-value"),
        ("x-api-key", "sk-ant-secret-value"),
        ("anthropic-version", "2023-06-01"),
        (
            "anthropic-beta",
            "oauth-2025-04-20,extended-cache-ttl-2025-04-11",
        ),
        ("anthropic-unknown-future", "yes"),
        ("x-claude-code-session-id", "sess-1"),
        ("x-claude-code-not-yet-invented", "1"),
        ("accept-encoding", "gzip, br"),
        ("x-multi", "one"),
        ("x-multi", "two"),
        ("connection", "keep-alive, x-hop"),
        ("x-hop", "private"),
        ("keep-alive", "timeout=5"),
    ];
    let answer = post_messages(proxy.addr, &headers, body).await;
    assert_eq!(answer.status, 200);

    let seen = &upstream.seen()[0];
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.uri, "/v1/messages");
    for (name, value) in &headers[..8] {
        // Everything passes unchanged except `accept-encoding`, which becomes `identity` so the
        // proxy can read the usage from an uncompressed response.
        let expected = if *name == "accept-encoding" {
            "identity"
        } else {
            *value
        };
        assert_eq!(seen.header(name), Some(expected), "{name}");
    }
    let multi: Vec<&str> = seen
        .headers
        .iter()
        .filter(|(n, _)| n == "x-multi")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(multi, ["one", "two"]);
    for hop in ["x-hop", "keep-alive"] {
        assert_eq!(seen.header(hop), None, "{hop}");
    }
    assert_eq!(
        seen.header("host"),
        Some(upstream.addr.to_string().as_str())
    );
    assert_eq!(
        seen.header("content-length"),
        Some(body.len().to_string().as_str())
    );
    assert_eq!(answer.header("request-id"), Some("req_x"));
    proxy.stop().await;
}

#[tokio::test]
async fn other_endpoints_are_forwarded_intact_and_not_recorded() {
    let upstream = FakeUpstream::always(Reply::json(200, r#"{"data":[]}"#)).await;
    let proxy = start(&upstream.url(), vec![]).await;
    let get = send(
        proxy.addr,
        Method::GET,
        "/v1/models?limit=1000",
        &[("x-api-key", "k")],
        "",
    )
    .await;
    assert_eq!(get.status, 200);
    assert_eq!(&get.body[..], br#"{"data":[]}"#);
    let count = br#"{ "model":"m", "messages":[] }"#;
    send(
        proxy.addr,
        Method::POST,
        "/v1/messages/count_tokens?beta=true",
        &[],
        &count[..],
    )
    .await;
    let head = send(proxy.addr, Method::HEAD, "/api/hello", &[], "").await;
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());

    let seen = upstream.seen();
    let uris: Vec<(&str, &str)> = seen
        .iter()
        .map(|s| (s.method.as_str(), s.uri.as_str()))
        .collect();
    assert_eq!(
        uris,
        [
            ("GET", "/v1/models?limit=1000"),
            ("POST", "/v1/messages/count_tokens?beta=true"),
            ("HEAD", "/api/hello"),
        ]
    );
    assert_eq!(&seen[1].body[..], &count[..]);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(proxy.sink.requests().is_empty());
    proxy.stop().await;
}

#[tokio::test]
async fn upstream_errors_pass_through_with_their_headers_and_bodies() {
    let cases = [
        (
            429,
            r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#,
        ),
        (
            500,
            r#"{"type":"error","error":{"type":"api_error","message":"boom"}}"#,
        ),
        (
            529,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        ),
        (
            400,
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"bound to a different conversation"}}"#,
        ),
    ];
    for (status, body) in cases {
        let reply = Reply::json(status, body)
            .header("retry-after", "7")
            .header("x-should-retry", "true")
            .header("anthropic-ratelimit-unified-status", "rejected")
            .header("anthropic-ratelimit-unified-reset", "1767225600")
            .header("request-id", "req_err");
        let upstream = FakeUpstream::always(reply).await;
        let proxy = start(&upstream.url(), vec![]).await;
        let answer = post_messages(
            proxy.addr,
            &[],
            br#"{"model":"claude-opus-5-5","messages":[]}"#,
        )
        .await;
        assert_eq!(answer.status, status);
        assert_eq!(&answer.body[..], body.as_bytes());
        for (name, value) in [
            ("retry-after", "7"),
            ("x-should-retry", "true"),
            ("anthropic-ratelimit-unified-status", "rejected"),
            ("anthropic-ratelimit-unified-reset", "1767225600"),
            ("content-type", "application/json"),
        ] {
            assert_eq!(answer.header(name), Some(value), "{status} {name}");
        }
        let record = &proxy.sink.wait_for(1).await[0];
        assert_eq!(record.status, status);
        assert_eq!(record.model_effective, "");
        assert_eq!(record.message_key, None);
        assert_eq!(
            upstream.seen().len(),
            1,
            "an unchanged request is never resent"
        );
        proxy.stop().await;
    }
}

#[tokio::test]
async fn messages_with_a_query_go_through_the_pipeline() {
    let upstream = FakeUpstream::always(ok_message()).await;
    let proxy = start(&upstream.url(), vec![]).await;
    let answer = send(
        proxy.addr,
        Method::POST,
        "/v1/messages?beta=true",
        &[("x-claude-code-session-id", "s9")],
        &br#"{"model":"claude-opus-5-5","messages":[{"role":"user","content":"title?"}]}"#[..],
    )
    .await;
    assert_eq!(answer.status, 200);
    assert_eq!(upstream.seen()[0].uri, "/v1/messages?beta=true");
    let record = &proxy.sink.wait_for(1).await[0];
    assert_eq!(record.session_id, "s9");
    assert_eq!(record.origin, "auxiliary", "no tools, one message");
    assert_eq!(record.message_key.as_deref(), Some("msg_x:req_x"));
    assert_eq!(record.usage.input + record.usage.output, 3);
    proxy.stop().await;
}

#[tokio::test]
async fn an_unreachable_upstream_gives_502_and_a_record() {
    let closed = closed_port().await;
    let (observer, calls) = hook(Behavior::Observe);
    let proxy = start(&format!("http://{closed}"), vec![observer]).await;
    let answer = post_messages(proxy.addr, &[], br#"{"model":"m","messages":[]}"#).await;
    assert_eq!(answer.status, 502);
    assert_eq!(answer.json()["error"]["type"], "api_error");
    let record = &proxy.sink.wait_for(1).await[0];
    assert_eq!(record.status, 502);
    assert_eq!(calls.after.lock().unwrap()[0].status, 502);

    let other = send(proxy.addr, Method::GET, "/v1/models", &[], "").await;
    assert_eq!(other.status, 502);
    assert_eq!(proxy.sink.requests().len(), 1);
    proxy.stop().await;
}

#[tokio::test]
async fn a_tls_failure_is_a_502() {
    // An https upstream pointed at a plain-text server: the handshake fails, no network used.
    let upstream = FakeUpstream::always(ok_message()).await;
    let proxy = start(&format!("https://{}", upstream.addr), vec![]).await;
    let answer = post_messages(proxy.addr, &[], b"{}").await;
    assert_eq!(answer.status, 502);
    proxy.stop().await;
}

#[tokio::test]
async fn a_body_that_is_not_json_is_forwarded_and_recorded() {
    let upstream = FakeUpstream::always(ok_message()).await;
    let (observer, calls) = hook(Behavior::Rewrite);
    let proxy = start(&upstream.url(), vec![observer]).await;
    let body = b"\x00\xffnot json at all";
    let answer = post_messages(proxy.addr, &[("x-claude-code-agent-id", "agent-7")], body).await;
    assert_eq!(answer.status, 200);
    assert_eq!(&upstream.seen()[0].body[..], body);
    let record = &proxy.sink.wait_for(1).await[0];
    assert_eq!(record.origin, "subagent");
    assert_eq!(record.agent_id.as_deref(), Some("agent-7"));
    assert_eq!(record.model_requested, "");
    assert_eq!(record.tools_hash, "");
    assert_eq!(record.bytes_in, body.len() as u64);
    assert!(calls.before.lock().unwrap().is_empty(), "no hook ran");
    assert!(calls.after.lock().unwrap().is_empty());
    proxy.stop().await;
}

#[tokio::test]
async fn a_request_body_that_breaks_off_gets_400_and_is_not_forwarded() {
    let upstream = FakeUpstream::always(ok_message()).await;
    let proxy = start(&upstream.url(), vec![]).await;
    let mut stream = tokio::net::TcpStream::connect(proxy.addr).await.unwrap();
    let head = format!(
        "POST /v1/messages HTTP/1.1\r\nhost: {}\r\ncontent-length: 100\r\n\r\n{{\"mo",
        proxy.addr
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.shutdown().await.unwrap();
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out).await;
    assert!(out.is_empty() || out.starts_with("HTTP/1.1 400"), "{out}");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(upstream.seen().is_empty());
    proxy.stop().await;
}

#[tokio::test]
async fn an_upstream_with_a_base_path_keeps_it() {
    let upstream = FakeUpstream::always(ok_message()).await;
    let proxy = start_with(
        support::config(&format!("{}/anthropic/", upstream.url())),
        vec![],
    )
    .await;
    send(proxy.addr, Method::GET, "/v1/models", &[], "").await;
    assert_eq!(upstream.seen()[0].uri, "/anthropic/v1/models");
    let raw_answer = raw(
        proxy.addr,
        format!(
            "GET /v1/models HTTP/1.1\r\nhost: localhost:{}\r\nconnection: close\r\n\r\n",
            proxy.addr.port()
        )
        .as_bytes(),
    )
    .await;
    assert!(raw_answer.starts_with("HTTP/1.1 200"), "{raw_answer}");
    proxy.stop().await;
}
