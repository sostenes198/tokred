//! The `ureq` adapters against a tiny local HTTP server (no external network).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

use tokred_core::pricing::SourceError;
use tokred_core::release::ReleaseSource;
use tokred_update::Downloader;
use tokred_update::http::{
    CHECK_TIMEOUT, DOWNLOAD_TIMEOUT, GithubReleaseSource, LATEST_RELEASE_URL, UreqDownloader,
};

/// Serves one response, then stops; the request line and headers are returned through the
/// channel. Returns the URL.
fn serve_once(status: &str, body: &[u8]) -> (String, std::sync::mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/asset", listener.local_addr().unwrap());
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    let (tx, rx) = std::sync::mpsc::channel();
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
        stream.write_all(&response).unwrap();
        let _ = tx.send(String::from_utf8_lossy(&seen).into_owned());
    });
    (url, rx)
}

fn refused_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    url
}

/// A server that accepts and never answers; returns the URL and a handle to release it.
fn stalled_server() -> (
    String,
    std::sync::mpsc::Sender<()>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let _ = rx.recv_timeout(Duration::from_secs(10));
        drop(stream);
    });
    (url, tx, handle)
}

#[test]
fn defaults_match_the_spec() {
    assert_eq!(
        LATEST_RELEASE_URL,
        "https://api.github.com/repos/sostenes198/tokred/releases/latest"
    );
    assert_eq!(CHECK_TIMEOUT, Duration::from_secs(2));
    assert!(DOWNLOAD_TIMEOUT >= Duration::from_secs(30));
    assert!(format!("{:?}", GithubReleaseSource::default()).contains("api.github.com"));
    assert!(format!("{:?}", UreqDownloader::default()).contains("timeout"));
}

#[test]
fn reads_the_latest_release() {
    let (url, request) = serve_once(
        "200 OK",
        br#"{"tag_name":"v0.4.0","html_url":"https://github.com/sostenes198/tokred/releases/tag/v0.4.0"}"#,
    );
    let release = GithubReleaseSource::new().with_url(url).latest().unwrap();
    assert_eq!(release.version, "0.4.0");
    assert!(release.url.ends_with("/v0.4.0"));
    let request = request
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        request.contains("accept: application/vnd.github+json"),
        "{request}"
    );
}

#[test]
fn release_errors_are_classified() {
    let (url, _) = serve_once("403 Forbidden", b"rate limited");
    let err = GithubReleaseSource::new()
        .with_url(url)
        .latest()
        .unwrap_err();
    assert!(matches!(err, SourceError::Unavailable(_)), "{err}");

    let (url, _) = serve_once("200 OK", b"{}");
    let err = GithubReleaseSource::new()
        .with_url(url)
        .latest()
        .unwrap_err();
    assert!(matches!(err, SourceError::Invalid(_)), "{err}");

    let err = GithubReleaseSource::new()
        .with_url(refused_url())
        .latest()
        .unwrap_err();
    assert!(matches!(err, SourceError::Unavailable(_)), "{err}");
}

#[test]
fn a_slow_release_server_hits_the_timeout() {
    let (url, release, handle) = stalled_server();
    let source = GithubReleaseSource::new()
        .with_url(url)
        .with_timeout(Duration::from_millis(200));
    let started = std::time::Instant::now();
    assert!(matches!(source.latest(), Err(SourceError::Unavailable(_))));
    assert!(started.elapsed() < Duration::from_secs(5));
    release.send(()).unwrap();
    handle.join().unwrap();
}

#[test]
fn downloads_bytes_unchanged() {
    let payload: Vec<u8> = (0..=255_u8).cycle().take(100_000).collect();
    let (url, _) = serve_once("200 OK", &payload);
    assert_eq!(UreqDownloader::new().download(&url).unwrap(), payload);
}

#[test]
fn download_errors_are_unavailable() {
    let (url, _) = serve_once("404 Not Found", b"nope");
    assert!(matches!(
        UreqDownloader::new().download(&url),
        Err(SourceError::Unavailable(_))
    ));
    assert!(matches!(
        UreqDownloader::new().download(&refused_url()),
        Err(SourceError::Unavailable(_))
    ));
}

#[test]
fn a_stalled_download_hits_the_timeout() {
    let (url, release, handle) = stalled_server();
    let downloader = UreqDownloader::new().with_timeout(Duration::from_millis(200));
    assert!(matches!(
        downloader.download(&url),
        Err(SourceError::Unavailable(_))
    ));
    release.send(()).unwrap();
    handle.join().unwrap();
}
