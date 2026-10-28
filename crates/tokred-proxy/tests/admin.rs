//! Host validation and the `/__tokred/*` endpoints.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hyper::Method;
use support::{FakeUpstream, Reply, SECRET, raw, send, start};
use tokred_proxy::{HEALTH_PATH, SECRET_HEADER, SHUTDOWN_PATH, STATUS_PATH, VERSION};

async fn upstream() -> FakeUpstream {
    FakeUpstream::always(Reply::json(200, "{}")).await
}

#[tokio::test]
async fn foreign_hosts_get_421_and_nothing_is_forwarded() {
    let upstream = upstream().await;
    let proxy = start(&upstream.url(), vec![]).await;
    let port = proxy.addr.port();
    for host in [
        format!("host: evil.example:{port}\r\n"),
        format!("host: 127.0.0.1:{}\r\n", port.wrapping_add(1)),
        String::new(),
    ] {
        let answer = raw(
            proxy.addr,
            format!("GET /v1/models HTTP/1.1\r\n{host}connection: close\r\n\r\n").as_bytes(),
        )
        .await;
        assert!(answer.starts_with("HTTP/1.1 421"), "{host:?}: {answer}");
        assert!(answer.contains("invalid_request_error"));
    }
    let admin = raw(
        proxy.addr,
        b"GET /__tokred/health HTTP/1.1\r\nhost: rebind.example\r\nconnection: close\r\n\r\n",
    )
    .await;
    assert!(admin.starts_with("HTTP/1.1 421"), "admin is protected too");
    let allowed = raw(
        proxy.addr,
        format!("GET /v1/models HTTP/1.1\r\nhost: [::1]:{port}\r\nconnection: close\r\n\r\n")
            .as_bytes(),
    )
    .await;
    assert!(allowed.starts_with("HTTP/1.1 200"), "{allowed}");
    assert_eq!(upstream.seen().len(), 1);
    proxy.stop().await;
}

#[tokio::test]
async fn health_needs_no_secret() {
    let upstream = upstream().await;
    let proxy = start(&upstream.url(), vec![]).await;
    let answer = send(proxy.addr, Method::GET, HEALTH_PATH, &[], "").await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.header("content-type"), Some("application/json"));
    assert_eq!(
        answer.json(),
        serde_json::json!({"ok": true, "version": VERSION})
    );
    proxy.stop().await;
}

#[tokio::test]
async fn status_and_shutdown_need_the_secret() {
    let upstream = upstream().await;
    let proxy = start(&upstream.url(), vec![]).await;
    for headers in [&[][..], &[(SECRET_HEADER, "wrong")][..]] {
        let status = send(proxy.addr, Method::GET, STATUS_PATH, headers, "").await;
        assert_eq!(status.status, 401);
        assert_eq!(status.json()["ok"], false);
        let stop = send(proxy.addr, Method::POST, SHUTDOWN_PATH, headers, "").await;
        assert_eq!(stop.status, 401);
    }
    assert!(!proxy.shutdown.is_triggered());

    let status = send(
        proxy.addr,
        Method::GET,
        STATUS_PATH,
        &[(SECRET_HEADER, SECRET)],
        "",
    )
    .await
    .json();
    assert_eq!(status["ok"], true);
    assert_eq!(status["version"], VERSION);
    assert_eq!(status["listen"], proxy.addr.to_string());
    assert_eq!(status["upstream"], upstream.url());
    assert!(status["requests"].as_u64().unwrap() >= 5);
    assert!(!status.to_string().contains(SECRET));

    let stop = send(
        proxy.addr,
        Method::POST,
        SHUTDOWN_PATH,
        &[(SECRET_HEADER, SECRET)],
        "",
    )
    .await;
    assert_eq!(stop.status, 200);
    assert_eq!(stop.json()["stopping"], true);
    assert!(proxy.shutdown.is_triggered());
    proxy.task.await.unwrap().unwrap();
    assert!(
        upstream.seen().is_empty(),
        "admin requests are never forwarded"
    );
}

#[tokio::test]
async fn unknown_admin_paths_and_methods_are_refused() {
    let upstream = upstream().await;
    let proxy = start(&upstream.url(), vec![]).await;
    let unknown = send(proxy.addr, Method::GET, "/__tokred/secrets", &[], "").await;
    assert_eq!(unknown.status, 404);
    let wrong = send(
        proxy.addr,
        Method::GET,
        SHUTDOWN_PATH,
        &[(SECRET_HEADER, SECRET)],
        "",
    )
    .await;
    assert_eq!(wrong.status, 405);
    let post_health = send(proxy.addr, Method::POST, HEALTH_PATH, &[], "").await;
    assert_eq!(post_health.status, 405);
    assert!(upstream.seen().is_empty());
    proxy.stop().await;
}

#[tokio::test]
async fn malformed_requests_do_not_hurt_the_proxy() {
    let upstream = upstream().await;
    let proxy = start(&upstream.url(), vec![]).await;
    let answer = raw(proxy.addr, b"\x16\x03\x01 not http at all\r\n\r\n").await;
    assert!(
        answer.is_empty() || answer.starts_with("HTTP/1.1 400"),
        "{answer}"
    );
    let health = send(proxy.addr, Method::GET, HEALTH_PATH, &[], "").await;
    assert_eq!(health.status, 200);
    proxy.stop().await;
}
