//! `LiteLlmSource` against a tiny local HTTP server (no external network).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use tokred_core::pricing::{PriceSource, SourceError};
use tokred_pricing::http::{DEFAULT_TIMEOUT, LITELLM_URL, LiteLlmSource};

/// Serves one response, then stops. Returns the URL.
fn serve_once(status: &str, body: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/prices.json", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut seen = Vec::new();
        let mut chunk = [0_u8; 512];
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            seen.extend_from_slice(&chunk[..n]);
        }
        stream.write_all(response.as_bytes()).unwrap();
    });
    url
}

const PRICES: &str = r#"{
    "sample_spec": {"mode": "chat"},
    "claude-sonnet-4-5": {"litellm_provider":"anthropic","mode":"chat",
        "input_cost_per_token":3e-6,"output_cost_per_token":1.5e-5,
        "cache_creation_input_token_cost":3.75e-6,"cache_read_input_token_cost":3e-7}
}"#;

#[test]
fn fetches_and_parses_prices() {
    let source = LiteLlmSource::new().with_url(serve_once("200 OK", PRICES));
    let entries = source.fetch().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].model, "claude-sonnet-4-5");
    assert_eq!(entries[0].price.output, 15.0);
}

#[test]
fn defaults_point_at_litellm_with_a_short_timeout() {
    assert!(LITELLM_URL.starts_with("https://raw.githubusercontent.com/BerriAI/litellm/"));
    assert!(DEFAULT_TIMEOUT <= Duration::from_secs(10));
    let source = LiteLlmSource::default();
    assert!(format!("{source:?}").contains("BerriAI"));
}

#[test]
fn http_errors_are_unavailable() {
    let source = LiteLlmSource::new().with_url(serve_once("503 Service Unavailable", "busy"));
    assert!(matches!(source.fetch(), Err(SourceError::Unavailable(_))));
}

#[test]
fn an_unexpected_body_is_invalid() {
    let source = LiteLlmSource::new().with_url(serve_once("200 OK", "<html>nope</html>"));
    let err = source.fetch().unwrap_err();
    assert!(matches!(err, SourceError::Invalid(_)), "{err}");

    let source = LiteLlmSource::new().with_url(serve_once("200 OK", r#"{"gpt-4":{}}"#));
    assert!(matches!(source.fetch(), Err(SourceError::Invalid(_))));
}

#[test]
fn a_refused_connection_is_unavailable() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    let source = LiteLlmSource::new().with_url(url);
    assert!(matches!(source.fetch(), Err(SourceError::Unavailable(_))));
}

#[test]
fn a_slow_server_hits_the_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        // Hold the connection open without answering until the client gave up.
        let _ = done_rx.recv_timeout(Duration::from_secs(10));
        drop(stream);
    });
    let source = LiteLlmSource::new()
        .with_url(url)
        .with_timeout(Duration::from_millis(200));
    let started = std::time::Instant::now();
    assert!(matches!(source.fetch(), Err(SourceError::Unavailable(_))));
    assert!(started.elapsed() < Duration::from_secs(5));
    done_tx.send(()).unwrap();
    handle.join().unwrap();
}
