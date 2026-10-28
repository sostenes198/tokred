//! Credentials and the admin secret never reach the logs or the records. This file installs a
//! global log subscriber, so it holds a single test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::io::Write;
use std::sync::{Arc, Mutex};

use hyper::Method;
use support::{
    Behavior, FakeUpstream, Reply, SECRET, closed_port, hook, main_body, post_messages, send,
    sse_body, start,
};
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

const API_KEY: &str = "sk-ant-api03-SECRETVALUE";
const OAUTH: &str = "Bearer sk-ant-oat01-OAUTHSECRET";
const COOKIE: &str = "session=COOKIESECRET";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credentials_never_reach_logs_or_records() {
    let capture = Capture::default();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(capture.clone())
        .init();

    let credentials = [
        ("x-api-key", API_KEY),
        ("authorization", OAUTH),
        ("cookie", COOKIE),
        ("anthropic-beta", "oauth-2025-04-20"),
        ("x-claude-code-session-id", "s1"),
        ("x-claude-code-agent-id", "a1"),
    ];

    // A failing hook, a compressed response and a normal one.
    let mut gzip = Reply::stream(vec![vec![0x1f, 0x8b, 0]]);
    gzip.headers
        .push(("content-encoding".into(), "gzip".into()));
    let upstream = FakeUpstream::start(Arc::new(move |seen| {
        if seen.uri.contains("gzip") {
            gzip.clone()
        } else {
            Reply::stream(vec![sse_body("msg_s", "claude-opus-5-5")])
        }
    }))
    .await;
    let (failing, _) = hook(Behavior::Fail);
    let proxy = start(&upstream.url(), vec![failing]).await;
    post_messages(
        proxy.addr,
        &credentials,
        &main_body("claude-opus-5-5", &["Read"]),
    )
    .await;
    send(
        proxy.addr,
        Method::POST,
        "/v1/messages?gzip=1",
        &credentials,
        main_body("claude-opus-5-5", &["Read"]),
    )
    .await;
    send(
        proxy.addr,
        Method::GET,
        "/__tokred/status",
        &[("x-tokred-secret", SECRET)],
        "",
    )
    .await;
    let mut records = proxy.sink.wait_for(2).await;
    proxy.stop().await;

    // An unreachable upstream (warns with the connection error).
    let closed = closed_port().await;
    let (panicking, _) = hook(Behavior::Panic);
    let proxy = start(&format!("http://{closed}"), vec![panicking]).await;
    post_messages(
        proxy.addr,
        &credentials,
        &main_body("claude-opus-5-5", &["Read"]),
    )
    .await;
    records.extend(proxy.sink.wait_for(1).await);
    proxy.stop().await;

    let logs = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    for expected in [
        "a proxy hook failed",
        "the upstream request failed",
        "compressed response",
        "proxy listening",
    ] {
        assert!(logs.contains(expected), "missing `{expected}` in:\n{logs}");
    }
    let recorded = format!("{records:?}");
    for secret in [
        API_KEY,
        OAUTH,
        COOKIE,
        "SECRETVALUE",
        "OAUTHSECRET",
        "COOKIESECRET",
        SECRET,
    ] {
        assert!(!logs.contains(secret), "`{secret}` leaked into the logs");
        assert!(
            !recorded.contains(secret),
            "`{secret}` leaked into a record"
        );
    }
}
