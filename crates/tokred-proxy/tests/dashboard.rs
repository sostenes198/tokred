//! The dashboard served by the proxy: routes, sign-in, the JSON API, the event stream, content
//! capture and shutdown, against the fake upstream.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use http_body_util::BodyExt;
use hyper::Method;
use hyper::body::Incoming;
use serde_json::{Value, json};
use support::{
    Answer, FakeSource, FakeUpstream, Reply, Running, SECRET, config, main_body, send, sse_events,
    start, start_dashboard,
};
use tokred_core::dashboard::{SessionList, SessionStatus, SessionView};

const SESSION: &str = "sess-1";

fn session_view(id: &str, last: i64, status: SessionStatus) -> SessionView {
    SessionView {
        id: id.to_owned(),
        started_at: last - 100,
        last_activity_at: last,
        status,
        requests: 7,
        models: vec!["claude-opus-5-5".to_owned()],
        cost_usd: 1.25,
        input: 1,
        output: 2,
        cache_read: 3,
        cache_write: 4,
        subagents_running: 0,
        content_recorded: false,
    }
}

async fn dashboard(upstream: &FakeUpstream, capture: bool) -> (Running, Arc<FakeSource>) {
    let source = Arc::new(FakeSource::default());
    let proxy = start_dashboard(config(&upstream.url()), vec![], source.clone(), capture).await;
    (proxy, source)
}

async fn quiet_upstream() -> FakeUpstream {
    FakeUpstream::always(Reply::json(200, "{}")).await
}

/// Signs in the way the CLI and the browser do: token from the admin endpoint, then the cookie.
async fn sign_in(proxy: &Running) -> String {
    let token = send(
        proxy.addr,
        Method::POST,
        "/__tokred/dashboard-token",
        &[("x-tokred-secret", SECRET)],
        "",
    )
    .await
    .json()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let answer = send(
        proxy.addr,
        Method::GET,
        &format!("/dashboard?token={token}"),
        &[],
        "",
    )
    .await;
    assert_eq!(answer.status, 303);
    assert_eq!(answer.header("location"), Some("/dashboard"));
    let cookie = answer.header("set-cookie").unwrap();
    assert!(
        cookie.starts_with("tokred_dash=")
            && cookie.contains("HttpOnly")
            && cookie.contains("SameSite=Strict")
            && cookie.contains("Path=/"),
        "{cookie}"
    );
    cookie.split(';').next().unwrap().to_owned()
}

async fn get(proxy: &Running, path: &str, cookie: &str) -> Answer {
    send(proxy.addr, Method::GET, path, &[("cookie", cookie)], "").await
}

fn assert_hardened(answer: &Answer) {
    assert!(answer.header("access-control-allow-origin").is_none());
    assert_eq!(answer.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(answer.header("referrer-policy"), Some("no-referrer"));
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    assert!(answer.header("content-security-policy").is_some());
}

#[tokio::test]
async fn every_route_needs_a_session_and_the_token_works_once() {
    let upstream = quiet_upstream().await;
    let (proxy, source) = dashboard(&upstream, false).await;
    *source.metrics.lock().unwrap() = Some("<html><body>report</body></html>".to_owned());

    for path in [
        "/dashboard",
        "/dashboard/app.css",
        "/dashboard/app.js",
        "/dashboard/api/sessions",
        "/dashboard/api/sessions/x",
        "/dashboard/events",
        "/metrics",
        "/favicon.ico",
    ] {
        let answer = send(proxy.addr, Method::GET, path, &[], "").await;
        assert_eq!(answer.status, 401, "{path}");
        assert!(String::from_utf8_lossy(&answer.body).contains("tokred dashboard"));
        assert_hardened(&answer);
        let wrong = get(&proxy, path, "tokred_dash=not-a-session").await;
        assert_eq!(wrong.status, 401, "{path}");
    }

    // The token endpoint needs the secret and only answers POST.
    let refused = send(
        proxy.addr,
        Method::POST,
        "/__tokred/dashboard-token",
        &[],
        "",
    )
    .await;
    assert_eq!(refused.status, 401);
    let wrong = [("x-tokred-secret", "nope")];
    let refused = send(
        proxy.addr,
        Method::POST,
        "/__tokred/dashboard-token",
        &wrong,
        "",
    )
    .await;
    assert_eq!(refused.status, 401);
    let secret = [("x-tokred-secret", SECRET)];
    let wrong_method = send(
        proxy.addr,
        Method::GET,
        "/__tokred/dashboard-token",
        &secret,
        "",
    )
    .await;
    assert_eq!(wrong_method.status, 405);

    let issued = send(
        proxy.addr,
        Method::POST,
        "/__tokred/dashboard-token",
        &secret,
        "",
    )
    .await;
    assert_eq!(issued.status, 200);
    assert_eq!(issued.json()["expires_in"], 60);
    let token = issued.json()["token"].as_str().unwrap().to_owned();
    assert_eq!(token.len(), 64);

    let wrong_token = send(proxy.addr, Method::GET, "/dashboard?token=0000", &[], "").await;
    assert_eq!(wrong_token.status, 401);
    let signed_in = send(
        proxy.addr,
        Method::GET,
        &format!("/dashboard?token={token}"),
        &[],
        "",
    )
    .await;
    assert_eq!(signed_in.status, 303);
    let again = send(
        proxy.addr,
        Method::GET,
        &format!("/dashboard?token={token}"),
        &[],
        "",
    )
    .await;
    assert_eq!(again.status, 401, "a token works once");
    // The token is only for the page: another route does not redeem it.
    let other = send(proxy.addr, Method::GET, "/metrics?token=whatever", &[], "").await;
    assert_eq!(other.status, 401);

    let cookie = signed_in
        .header("set-cookie")
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let page = get(&proxy, "/dashboard", &cookie).await;
    assert_eq!(page.status, 200);
    assert_eq!(
        page.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    let html = String::from_utf8_lossy(&page.body).into_owned();
    assert!(html.contains("/dashboard/app.css") && html.contains("/dashboard/app.js"));
    assert_hardened(&page);
    assert_eq!(
        page.header("content-security-policy"),
        Some(
            "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'"
        )
    );
    // A cookie among others, and a valid token in the URL of a signed-in browser.
    let mixed = format!("theme=dark; {cookie}; other=1");
    assert_eq!(get(&proxy, "/dashboard", &mixed).await.status, 200);

    let css = get(&proxy, "/dashboard/app.css", &cookie).await;
    assert_eq!(
        (css.status, css.header("content-type")),
        (200, Some("text/css; charset=utf-8"))
    );
    let js = get(&proxy, "/dashboard/app.js", &cookie).await;
    assert_eq!(
        (js.status, js.header("content-type")),
        (200, Some("text/javascript; charset=utf-8"))
    );

    let report = get(&proxy, "/metrics", &cookie).await;
    assert_eq!(report.status, 200);
    assert_eq!(&report.body[..], b"<html><body>report</body></html>");
    let csp = report.header("content-security-policy").unwrap();
    assert!(
        csp.contains("style-src 'unsafe-inline'") && csp.contains("script-src 'none'"),
        "{csp}"
    );
    assert_hardened(&report);

    let favicon = get(&proxy, "/favicon.ico", &cookie).await;
    assert_eq!(favicon.status, 204);
    assert!(favicon.body.is_empty());
    let head = send(
        proxy.addr,
        Method::HEAD,
        "/dashboard/favicon.ico",
        &[("cookie", &cookie)],
        "",
    )
    .await;
    assert_eq!(head.status, 204);

    // None of it reached the upstream.
    assert!(upstream.seen().is_empty());
    proxy.stop().await;
}

#[tokio::test]
async fn methods_other_origins_and_unknown_paths_are_refused() {
    let upstream = quiet_upstream().await;
    let (proxy, _) = dashboard(&upstream, false).await;
    let cookie = sign_in(&proxy).await;

    for method in [
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::OPTIONS,
        Method::HEAD,
    ] {
        for path in [
            "/dashboard",
            "/dashboard/api/sessions",
            "/metrics",
            "/dashboard/events",
        ] {
            // 405 comes before the sign-in check, and a cookie does not change it.
            let answer = send(proxy.addr, method.clone(), path, &[], "").await;
            assert_eq!(answer.status, 405, "{method} {path}");
            assert_eq!(answer.header("allow"), Some("GET"));
            let answer = send(proxy.addr, method.clone(), path, &[("cookie", &cookie)], "").await;
            assert_eq!(answer.status, 405, "{method} {path}");
        }
    }
    let post_favicon = send(proxy.addr, Method::POST, "/favicon.ico", &[], "").await;
    assert_eq!(post_favicon.header("allow"), Some("GET, HEAD"));

    for site in ["cross-site", "same-site"] {
        let headers = [("cookie", cookie.as_str()), ("sec-fetch-site", site)];
        let answer = send(
            proxy.addr,
            Method::GET,
            "/dashboard/api/sessions",
            &headers,
            "",
        )
        .await;
        assert_eq!(answer.status, 403, "{site}");
        assert_hardened(&answer);
    }
    let same = [
        ("cookie", cookie.as_str()),
        ("sec-fetch-site", "same-origin"),
    ];
    assert_eq!(
        send(
            proxy.addr,
            Method::GET,
            "/dashboard/api/sessions",
            &same,
            ""
        )
        .await
        .status,
        200
    );
    let typed = [("cookie", cookie.as_str()), ("sec-fetch-site", "none")];
    assert_eq!(
        send(proxy.addr, Method::GET, "/dashboard", &typed, "")
            .await
            .status,
        200
    );

    let missing = get(&proxy, "/dashboard/nope", &cookie).await;
    assert_eq!(missing.status, 404);
    assert_eq!(
        get(&proxy, "/dashboard/api/sessions/a/b", &cookie)
            .await
            .status,
        404
    );

    // The Host check still comes first.
    let evil = [("host", "evil.example"), ("cookie", cookie.as_str())];
    let answer = send(proxy.addr, Method::GET, "/dashboard", &evil, "").await;
    assert_eq!(answer.status, 421);
    assert!(upstream.seen().is_empty());
    proxy.stop().await;
}

#[tokio::test]
async fn without_a_dashboard_the_paths_are_forwarded_and_the_token_endpoint_is_absent() {
    let upstream = quiet_upstream().await;
    let proxy = start(&upstream.url(), vec![]).await;
    assert_eq!(
        send(proxy.addr, Method::GET, "/dashboard", &[], "")
            .await
            .status,
        200
    );
    assert_eq!(upstream.seen().len(), 1, "forwarded like any other request");
    let token = send(
        proxy.addr,
        Method::POST,
        "/__tokred/dashboard-token",
        &[("x-tokred-secret", SECRET)],
        "",
    )
    .await;
    assert_eq!(token.status, 404);
    proxy.stop().await;
}

#[tokio::test]
async fn the_api_serves_the_store_and_reports_its_failures() {
    let upstream = quiet_upstream().await;
    let (proxy, source) = dashboard(&upstream, false).await;
    let cookie = sign_in(&proxy).await;
    *source.sessions.lock().unwrap() = SessionList {
        active: vec![session_view("a", 4_000_000_000, SessionStatus::Active)],
        recent: vec![session_view("r", 1_000, SessionStatus::Ended)],
    };
    source.details.lock().unwrap().insert(
        "r".to_owned(),
        tokred_core::dashboard::SessionDetail {
            session: session_view("r", 1_000, SessionStatus::Ended),
            turns: vec![],
            truncated: false,
        },
    );

    let list = get(&proxy, "/dashboard/api/sessions", &cookie).await;
    assert_eq!(list.status, 200);
    assert_eq!(list.header("content-type"), Some("application/json"));
    assert_hardened(&list);
    let list = list.json();
    assert_eq!(list["active"][0]["id"], "a");
    assert_eq!(list["recent"][0]["id"], "r");
    assert_eq!(list["recent"][0]["status"], "ended");

    let detail = get(&proxy, "/dashboard/api/sessions/r", &cookie)
        .await
        .json();
    assert_eq!(detail["session"]["id"], "r");
    assert_eq!(detail["turns"], json!([]));
    assert!(detail.get("truncated").is_none());
    let unknown = get(&proxy, "/dashboard/api/sessions/nobody", &cookie).await;
    assert_eq!(unknown.status, 404);
    assert_eq!(unknown.json()["error"], "unknown session");

    source
        .failing
        .store(true, std::sync::atomic::Ordering::SeqCst);
    for path in ["/dashboard/api/sessions", "/dashboard/api/sessions/r"] {
        let answer = get(&proxy, path, &cookie).await;
        assert_eq!(answer.status, 500, "{path}");
        assert_eq!(answer.json()["error"], "the data could not be read");
        assert!(!String::from_utf8_lossy(&answer.body).contains("locked"));
    }
    let report = get(&proxy, "/metrics", &cookie).await;
    assert_eq!(report.status, 500, "the fake has no report");
    proxy.stop().await;
}

/// A reader of a `text/event-stream` response.
struct Events {
    body: Incoming,
    buffer: String,
}

#[derive(Debug)]
struct Event {
    id: u64,
    name: String,
    data: Value,
}

async fn open_events(proxy: &Running, query: &str, cookie: &str) -> Events {
    let response = support::client()
        .request(support::request(
            proxy.addr,
            Method::GET,
            &format!("/dashboard/events{query}"),
            &[("cookie", cookie)],
            "",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert!(
        response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
    assert_eq!(response.headers()["cache-control"], "no-store");
    Events {
        body: response.into_body(),
        buffer: String::new(),
    }
}

impl Events {
    /// The next event; `None` when the stream ended.
    async fn next(&mut self) -> Option<Event> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let block: String = self.buffer.drain(..end + 2).collect();
                let mut id = None;
                let (mut name, mut data) = (String::new(), String::new());
                for line in block.lines() {
                    if let Some(value) = line.strip_prefix("id: ") {
                        id = value.parse().ok();
                    } else if let Some(value) = line.strip_prefix("event: ") {
                        value.clone_into(&mut name);
                    } else if let Some(value) = line.strip_prefix("data: ") {
                        value.clone_into(&mut data);
                    }
                }
                let Some(id) = id else { continue };
                return Some(Event {
                    id,
                    name,
                    data: serde_json::from_str(&data).unwrap(),
                });
            }
            let step = tokio::time::timeout(Duration::from_secs(10), self.body.frame())
                .await
                .unwrap_or_else(|_| panic!("no event within 10 s; buffer: {:?}", self.buffer));
            match step {
                Some(Ok(frame)) => {
                    if let Some(data) = frame.data_ref() {
                        self.buffer.push_str(&String::from_utf8_lossy(data));
                    }
                }
                _ => return None,
            }
        }
    }

    /// Events up to and including the first one called `name`.
    async fn until(&mut self, name: &str) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let event = self
                .next()
                .await
                .unwrap_or_else(|| panic!("ended before {name}: {seen:?}"));
            let done = event.name == name;
            seen.push(event);
            if done {
                return seen;
            }
        }
    }
}

fn text_stream(id: &str) -> Vec<Vec<u8>> {
    let events = [
        format!(
            r#"{{"type":"message_start","message":{{"id":"{id}","model":"claude-opus-5-5","content":[],"usage":{{"input_tokens":12,"output_tokens":1,"cache_creation_input_tokens":300,"cache_read_input_tokens":4000}}}}}}"#
        ),
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#.to_owned(),
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hm"}}"#.to_owned(),
        r#"{"type":"content_block_stop","index":0}"#.to_owned(),
        r#"{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#.to_owned(),
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hel"}}"#.to_owned(),
        r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"lo"}}"#.to_owned(),
        r#"{"type":"content_block_stop","index":1}"#.to_owned(),
        r#"{"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t1","name":"Read","input":{}}}"#.to_owned(),
        r#"{"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\":\"/src/main.rs\"}"}}"#.to_owned(),
        r#"{"type":"content_block_stop","index":2}"#.to_owned(),
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":42}}"#.to_owned(),
        r#"{"type":"message_stop"}"#.to_owned(),
    ];
    events
        .iter()
        .map(|data| {
            let name = serde_json::from_str::<Value>(data).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned();
            format!("event: {name}\ndata: {data}\n\n").into_bytes()
        })
        .collect()
}

fn headers(session: &str) -> [(&'static str, &str); 1] {
    [("x-claude-code-session-id", session)]
}

#[tokio::test]
async fn a_streamed_turn_arrives_in_order_with_its_text() {
    let mut reply = Reply::stream(text_stream("msg_1"));
    reply.gap = Duration::from_millis(30);
    let upstream = FakeUpstream::always(reply).await;
    let (proxy, _source) = dashboard(&upstream, true).await;
    let cookie = sign_in(&proxy).await;
    let mut events = open_events(&proxy, "", &cookie).await;

    let addr = proxy.addr;
    let post = tokio::spawn(async move {
        support::post_messages(
            addr,
            &headers(SESSION),
            &main_body("claude-opus-5-5", &["Read"]),
        )
        .await
    });
    let seen = events.until("turn_end").await;
    assert_eq!(post.await.unwrap().status, 200);

    let ids: Vec<u64> = seen.iter().map(|e| e.id).collect();
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "{ids:?}");
    let names: Vec<&str> = seen.iter().map(|e| e.name.as_str()).collect();
    let start = names.iter().position(|n| *n == "turn_start").unwrap();
    let end = names.iter().position(|n| *n == "turn_end").unwrap();
    assert!(start < end, "{names:?}");
    assert!(
        names[start + 1..end]
            .iter()
            .all(|n| matches!(*n, "turn_delta" | "session")),
        "{names:?}"
    );

    let started = &seen[start].data;
    assert_eq!(started["in_flight"], true);
    assert_eq!(started["session_id"], SESSION);
    assert_eq!(started["origin"], "main");
    assert_eq!(started["model_requested"], "claude-opus-5-5");
    assert_eq!(started["response"], Value::Null);
    let sent = started["sent"].as_array().unwrap();
    assert_eq!(sent[0]["role"], "system");
    assert_eq!(sent[0]["text"], "You are Claude Code.");
    assert!(
        sent.iter()
            .any(|m| m["text"].as_str().unwrap().contains("find the bug"))
    );
    assert!(sent.iter().any(|m| {
        m["text"]
            .as_str()
            .unwrap()
            .contains("[tool_result] fn main() {}")
    }));

    let text: String = seen
        .iter()
        .filter(|e| e.name == "turn_delta")
        .map(|e| {
            assert_eq!(e.data["session_id"], SESSION);
            assert_eq!(e.data["turn_id"], started["id"]);
            e.data["text"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(text, "Hello");

    let finished = &seen[end].data;
    assert_eq!(finished["id"], started["id"]);
    assert_eq!(finished["in_flight"], false);
    assert_eq!(finished["status"], 200);
    assert_eq!(finished["stop_reason"], "tool_use");
    assert_eq!(finished["tools"], json!(["Read"]));
    assert_eq!(finished["model_effective"], "claude-opus-5-5");
    assert_eq!(
        finished["usage"],
        json!({"input": 12, "output": 42, "cache_write": 300, "cache_read": 4000})
    );
    assert!((finished["cost_usd"].as_f64().unwrap() - 0.042).abs() < 1e-9);
    assert!(finished["duration_ms"].is_u64() && finished["ttfb_ms"].is_u64());
    assert_eq!(finished["response"]["text"], "Hello");
    assert_eq!(finished["response"]["thinking"], "hm");
    assert_eq!(
        finished["response"]["tool_uses"],
        json!([{"name": "Read", "input_summary": "/src/main.rs"}])
    );
    assert_eq!(finished["sent"], started["sent"]);

    // The session event follows, merged with the live state.
    let session = events.until("session").await.pop().unwrap();
    assert_eq!(session.data["id"], SESSION);
    assert_eq!(session.data["status"], "active");
    assert_eq!(session.data["content_recorded"], true);
    proxy.stop().await;
}

#[tokio::test]
async fn the_stream_can_be_limited_to_one_session() {
    let upstream = FakeUpstream::always(Reply::stream(text_stream("msg_f"))).await;
    let (proxy, _) = dashboard(&upstream, false).await;
    let cookie = sign_in(&proxy).await;
    let mut only_b = open_events(&proxy, "?session=sess-b", &cookie).await;
    let body = main_body("claude-opus-5-5", &["Read"]);
    support::post_messages(proxy.addr, &headers("sess-a"), &body).await;
    support::post_messages(proxy.addr, &headers("sess-b"), &body).await;
    let seen = only_b.until("turn_end").await;
    assert!(seen.iter().all(|e| {
        e.data.get("session_id").is_none_or(|s| s == "sess-b")
            && e.data
                .get("id")
                .is_none_or(|id| id == "sess-b" || id.as_str().unwrap().len() == 16)
    }));
    assert_eq!(
        seen.iter().filter(|e| e.name == "turn_start").count(),
        1,
        "{seen:?}"
    );
    proxy.stop().await;
}

#[tokio::test]
async fn without_content_recording_nothing_about_content_is_read_sent_or_stored() {
    let upstream = FakeUpstream::always(Reply::stream(text_stream("msg_p"))).await;
    let (proxy, _) = dashboard(&upstream, false).await;
    let cookie = sign_in(&proxy).await;
    let mut events = open_events(&proxy, "", &cookie).await;
    let answer = support::post_messages(
        proxy.addr,
        &headers(SESSION),
        &main_body("claude-opus-5-5", &["Read"]),
    )
    .await;
    assert_eq!(answer.status, 200);
    let seen = events.until("turn_end").await;
    assert!(
        seen.iter().all(|e| e.name != "turn_delta"),
        "a delta is content"
    );
    let start = seen.iter().find(|e| e.name == "turn_start").unwrap();
    assert_eq!(start.data["sent"], Value::Null);
    let end = seen.last().unwrap();
    assert_eq!(end.data["response"], Value::Null);
    assert_eq!(end.data["sent"], Value::Null);
    assert_eq!(
        end.data["tools"],
        json!(["Read"]),
        "tool names are metadata"
    );
    assert_eq!(end.data["stop_reason"], "tool_use");
    assert_eq!(end.data["usage"]["output"], 42);

    let captured = proxy.sink.wait_for_captured(1).await;
    assert_eq!(captured[0].sent_json, None);
    assert_eq!(captured[0].response_json, None);
    assert_eq!(captured[0].tools_json, r#"["Read"]"#);
    assert_eq!(captured[0].stop_reason.as_deref(), Some("tool_use"));
    assert_eq!(captured[0].session_id, SESSION);
    let record = proxy.sink.wait_for(1).await;
    assert_eq!(captured[0].request_id, record[0].id);
    proxy.stop().await;
}

#[tokio::test]
async fn captured_turns_hold_only_what_is_new_and_the_reassembled_response() {
    let upstream = FakeUpstream::always(Reply::stream(text_stream("msg_c"))).await;
    let (proxy, _) = dashboard(&upstream, true).await;

    let first = main_body("claude-opus-5-5", &["Read"]);
    support::post_messages(proxy.addr, &headers(SESSION), &first).await;
    let mut bigger: Value = serde_json::from_slice(&first).unwrap();
    let messages = bigger["messages"].as_array_mut().unwrap();
    messages.push(json!({"role": "assistant", "content": [{"type": "text", "text": "Hello"}]}));
    messages.push(json!({"role": "user", "content": "now fix it"}));
    support::post_messages(
        proxy.addr,
        &headers(SESSION),
        &serde_json::to_vec(&bigger).unwrap(),
    )
    .await;

    let turns = proxy.sink.wait_for_captured(2).await;
    let sent = |turn: &tokred_core::dashboard::CapturedTurn| -> Vec<Value> {
        serde_json::from_str(turn.sent_json.as_ref().unwrap()).unwrap()
    };
    let (one, two) = (sent(&turns[0]), sent(&turns[1]));
    assert_eq!(one.len(), 4, "system prompt and the three messages");
    let texts: Vec<&str> = two.iter().map(|m| m["text"].as_str().unwrap()).collect();
    assert_eq!(
        texts,
        ["Hello", "now fix it"],
        "only what is new, no system prompt again"
    );
    let response: Value = serde_json::from_str(turns[1].response_json.as_ref().unwrap()).unwrap();
    assert_eq!(response["text"], "Hello");
    assert_eq!(response["thinking"], "hm");
    assert_eq!(response["tool_uses"][0]["input_summary"], "/src/main.rs");
    assert_eq!(turns[0].tools_json, r#"["Read"]"#);

    // Headers and credentials are never part of a record.
    let dump = format!("{turns:?}");
    assert!(!dump.contains("authorization") && !dump.contains(SECRET));
    proxy.stop().await;
}

#[tokio::test]
async fn a_turn_in_flight_shows_up_in_the_api_with_its_text_so_far() {
    // A request whose response is slow enough to look at.
    let mut slow = Reply::stream(text_stream("msg_slow"));
    slow.gap = Duration::from_millis(250);
    let slow_upstream = FakeUpstream::always(slow).await;
    let (slow_proxy, _) = dashboard(&slow_upstream, true).await;
    let slow_cookie = sign_in(&slow_proxy).await;
    let addr = slow_proxy.addr;
    let post = tokio::spawn(async move {
        support::post_messages(
            addr,
            &headers("live-1"),
            &main_body("claude-opus-5-5", &["Read"]),
        )
        .await
    });

    let mut listing = Value::Null;
    for _ in 0..100 {
        listing = get(&slow_proxy, "/dashboard/api/sessions", &slow_cookie)
            .await
            .json();
        if !listing["active"].as_array().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(listing["active"][0]["id"], "live-1", "{listing}");
    assert_eq!(listing["active"][0]["status"], "active");
    assert_eq!(listing["active"][0]["requests"], 0);

    let mut detail = Value::Null;
    for _ in 0..100 {
        detail = get(&slow_proxy, "/dashboard/api/sessions/live-1", &slow_cookie)
            .await
            .json();
        if detail["turns"][0]["response"]["text"] == "Hello" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let turn = &detail["turns"][0];
    assert_eq!(turn["in_flight"], true, "{detail}");
    assert_eq!(turn["response"]["text"], "Hello", "the text so far");
    assert_eq!(turn["status"], Value::Null);
    assert_eq!(detail["session"]["content_recorded"], true);

    assert_eq!(post.await.unwrap().status, 200);
    let done = get(&slow_proxy, "/dashboard/api/sessions/live-1", &slow_cookie)
        .await
        .json();
    assert_eq!(done["turns"][0]["in_flight"], false);
    assert_eq!(done["turns"][0]["tools"], json!(["Read"]));
    assert_eq!(
        done["session"]["requests"], 1,
        "a finished turn the store may not have yet"
    );
    assert_eq!(done["session"]["status"], "active");

    slow_proxy.stop().await;
}

#[tokio::test]
async fn a_failed_upstream_still_ends_the_turn() {
    let closed = support::closed_port().await;
    let source = Arc::new(FakeSource::default());
    let proxy = start_dashboard(config(&format!("http://{closed}")), vec![], source, true).await;
    let cookie = sign_in(&proxy).await;
    let mut events = open_events(&proxy, "", &cookie).await;
    let answer = support::post_messages(
        proxy.addr,
        &headers(SESSION),
        &main_body("claude-opus-5-5", &["Read"]),
    )
    .await;
    assert_eq!(answer.status, 502);
    let seen = events.until("turn_end").await;
    let end = seen.last().unwrap();
    assert_eq!(end.data["status"], 502);
    assert_eq!(end.data["in_flight"], false);
    assert_eq!(end.data["cost_usd"], Value::Null);
    assert_eq!(end.data["tools"], json!([]));
    proxy.stop().await;
}

#[tokio::test]
async fn subagents_and_requests_without_a_session_are_followed_or_ignored() {
    let upstream = FakeUpstream::always(Reply::stream(sse_events_bytes("msg_s"))).await;
    let (proxy, _) = dashboard(&upstream, false).await;
    let cookie = sign_in(&proxy).await;
    let mut events = open_events(&proxy, "", &cookie).await;

    let body = main_body("claude-opus-5-5", &["Read"]);
    // No session id: not followed (and no event for it).
    support::post_messages(proxy.addr, &[], &body).await;
    let sub = [
        ("x-claude-code-session-id", SESSION),
        ("x-claude-code-agent-id", "agent-7"),
        ("x-claude-code-agent-type", "Explore"),
    ];
    support::post_messages(proxy.addr, &sub, &body).await;

    let seen = events.until("turn_end").await;
    let running = seen.iter().find(|e| e.name == "subagent").unwrap();
    assert_eq!(running.data["state"], "running");
    assert_eq!(running.data["agent_id"], "agent-7");
    assert_eq!(running.data["agent_type"], "Explore");
    assert_eq!(running.data["session_id"], SESSION);
    assert!(running.data["ts"].is_i64());
    let end = seen.last().unwrap();
    assert_eq!(end.data["origin"], "subagent");
    assert_eq!(end.data["agent_id"], "agent-7");
    assert_eq!(
        end.data["session_id"], SESSION,
        "the request without a session was skipped"
    );

    let cookie_get = get(&proxy, "/dashboard/api/sessions", &cookie).await.json();
    assert_eq!(cookie_get["active"][0]["subagents_running"], 1);
    proxy.stop().await;
}

fn sse_events_bytes(id: &str) -> Vec<Vec<u8>> {
    sse_events(id, "claude-opus-5-5")
        .into_iter()
        .map(String::into_bytes)
        .collect()
}

#[tokio::test]
async fn shutdown_ends_open_event_streams_at_once() {
    let upstream = quiet_upstream().await;
    let (proxy, _) = dashboard(&upstream, false).await;
    let cookie = sign_in(&proxy).await;
    let mut one = open_events(&proxy, "", &cookie).await;
    let mut two = open_events(&proxy, "?session=x", &cookie).await;
    let started = std::time::Instant::now();
    proxy.shutdown.trigger();
    tokio::time::timeout(Duration::from_secs(3), proxy.task)
        .await
        .expect("the proxy waited for an open event stream")
        .unwrap()
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(one.next().await.is_none(), "the stream ended");
    assert!(two.next().await.is_none());
}
