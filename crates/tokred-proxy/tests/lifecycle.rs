//! Binding rules, the blocking wrappers and graceful shutdown.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use support::{FakeUpstream, MemorySink, Reply, SECRET, config, main_body, start_with};
use tokred_proxy::{Proxy, ProxyError, ServerConfig, Shutdown, run, run_blocking};

#[test]
fn binding_outside_loopback_needs_the_flag() {
    let mut config = config("https://api.anthropic.com");
    config.bind = "0.0.0.0:0".parse().unwrap();
    assert!(matches!(
        Proxy::bind(config),
        Err(ProxyError::NonLoopback(_))
    ));
}

#[test]
fn a_bad_upstream_or_a_busy_port_is_refused() {
    assert!(matches!(
        Proxy::bind(config("http://example.com")),
        Err(ProxyError::InvalidUpstream { .. })
    ));
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut busy = config("https://api.anthropic.com");
    busy.bind = taken.local_addr().unwrap();
    assert!(matches!(Proxy::bind(busy), Err(ProxyError::Bind { .. })));
}

fn blocking_get(addr: std::net::SocketAddr, request: &str) -> String {
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).unwrap();
    out
}

#[test]
fn the_blocking_server_runs_until_the_admin_shutdown() {
    let proxy = Proxy::bind(config("https://api.anthropic.com")).unwrap();
    let addr = proxy.local_addr();
    let shutdown = Shutdown::new();
    let server = {
        let shutdown = shutdown.clone();
        std::thread::spawn(move || {
            proxy.serve_blocking(Vec::new(), Arc::new(MemorySink::default()), shutdown)
        })
    };
    let health = blocking_get(
        addr,
        &format!(
            "GET /__tokred/health HTTP/1.1\r\nhost: localhost:{}\r\nconnection: close\r\n\r\n",
            addr.port()
        ),
    );
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    let stop = blocking_get(
        addr,
        &format!(
            "POST /__tokred/shutdown HTTP/1.1\r\nhost: 127.0.0.1:{}\r\nx-tokred-secret: {SECRET}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
            addr.port()
        ),
    );
    assert!(stop.starts_with("HTTP/1.1 200"), "{stop}");
    server.join().unwrap().unwrap();
    assert!(shutdown.is_triggered());
}

#[test]
fn run_blocking_returns_once_triggered_and_reports_bad_configs() {
    let shutdown = Shutdown::new();
    shutdown.trigger();
    let sink = Arc::new(MemorySink::default());
    run_blocking(
        config("https://api.anthropic.com"),
        Vec::new(),
        sink.clone(),
        shutdown.clone(),
    )
    .unwrap();
    let bad = ServerConfig::new("127.0.0.1:0".parse().unwrap(), "nope", "");
    assert!(run_blocking(bad, Vec::new(), sink, shutdown).is_err());
}

#[tokio::test]
async fn run_returns_once_triggered() {
    let shutdown = Shutdown::new();
    shutdown.trigger();
    run(
        config("https://api.anthropic.com"),
        Vec::new(),
        Arc::new(MemorySink::default()),
        shutdown,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn shutdown_lets_responses_in_flight_finish_up_to_the_drain_timeout() {
    let events: Vec<Vec<u8>> = support::sse_events("msg_long", "claude-opus-5-5")
        .into_iter()
        .map(String::into_bytes)
        .collect();
    let mut reply = Reply::stream(events);
    reply.gap = Duration::from_millis(300);
    let upstream = FakeUpstream::always(reply).await;
    let mut config = config(&upstream.url());
    config.drain_timeout = Duration::from_millis(200);
    let proxy = start_with(config, vec![]).await;

    let addr = proxy.addr;
    let client = tokio::spawn(async move {
        support::client()
            .request(support::request(
                addr,
                hyper::Method::POST,
                "/v1/messages",
                &[],
                main_body("claude-opus-5-5", &["Read"]),
            ))
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = Instant::now();
    proxy.shutdown.trigger();
    proxy.task.await.unwrap().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the drain is bounded"
    );
    let _ = client.await;
}

#[tokio::test]
async fn an_idle_keep_alive_connection_closes_on_shutdown() {
    let upstream = FakeUpstream::always(Reply::json(200, "{}")).await;
    let proxy = start_with(config(&upstream.url()), vec![]).await;
    let mut stream = tokio::net::TcpStream::connect(proxy.addr).await.unwrap();
    stream
        .write_all(
            format!(
                "GET /__tokred/health HTTP/1.1\r\nhost: localhost:{}\r\n\r\n",
                proxy.addr.port()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut first = [0_u8; 512];
    let read = stream.read(&mut first).await.unwrap();
    assert!(String::from_utf8_lossy(&first[..read]).starts_with("HTTP/1.1 200"));
    let started = Instant::now();
    proxy.shutdown.trigger();
    proxy.task.await.unwrap().unwrap();
    let mut rest = Vec::new();
    let _ = stream.read_to_end(&mut rest).await;
    assert!(started.elapsed() < Duration::from_secs(2));
}
