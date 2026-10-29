//! The proxy behind a corporate network: an HTTP proxy that tunnels with `CONNECT`, and an
//! upstream whose certificate is signed by a private authority. Everything runs on 127.0.0.1.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hyper::Method;
use support::net::{Behavior, FakeProxy, Pki, TlsUpstream, UPSTREAM_BODY, UPSTREAM_NAME, settings};
use support::{closed_port, config, send, start_with};
use tempfile::TempDir;

/// The proxy listening on `proxy`, with these environment variables and `ca_bundle`.
async fn run(upstream: &str, vars: &[(&str, &str)], ca_bundle: Option<&str>) -> support::Running {
    let mut config = config(upstream);
    config.network = settings(vars, ca_bundle);
    start_with(config, Vec::new()).await
}

fn ca_file(pki: &Pki) -> (TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = pki.write_ca(dir.path(), "corp-ca.pem");
    (dir, path.to_str().unwrap().to_owned())
}

#[tokio::test]
async fn a_request_goes_through_the_proxy_and_trusts_the_custom_ca() {
    let pki = Pki::new();
    let upstream = TlsUpstream::start(&pki).await;
    let corp = FakeProxy::start(Behavior::Tunnel, upstream.addr).await;
    let (_dir, ca) = ca_file(&pki);
    let proxy_url = corp.url().replace("http://", "http://alice:s3cret@");
    let proxy = run(
        &upstream.named_url(),
        &[("HTTPS_PROXY", &proxy_url), ("NODE_EXTRA_CA_CERTS", &ca)],
        None,
    )
    .await;

    let answer = send(proxy.addr, Method::GET, "/v1/models?x=1", &[], "").await;
    assert_eq!(answer.status, 200);
    assert_eq!(String::from_utf8_lossy(&answer.body), UPSTREAM_BODY);
    assert_eq!(upstream.served(), 1);

    let heads = corp.heads();
    assert_eq!(heads.len(), 1);
    let expected = format!("CONNECT {UPSTREAM_NAME}:{} HTTP/1.1", upstream.addr.port());
    assert!(heads[0].starts_with(&expected), "{}", heads[0]);
    assert!(
        heads[0].contains("YWxpY2U6czNjcmV0"),
        "the credentials of the proxy address are sent as basic auth"
    );

    // A second request reuses the tunnel.
    let again = send(proxy.addr, Method::GET, "/v1/models", &[], "").await;
    assert_eq!(again.status, 200);
    assert_eq!(corp.heads().len(), 1);
    proxy.stop().await;
}

#[tokio::test]
async fn the_ca_bundle_setting_and_ssl_cert_file_work_like_the_variable() {
    let pki = Pki::new();
    let upstream = TlsUpstream::start(&pki).await;
    let corp = FakeProxy::start(Behavior::Tunnel, upstream.addr).await;
    let (_dir, ca) = ca_file(&pki);
    let url = upstream.named_url();

    let by_config = run(&url, &[("https_proxy", &corp.url())], Some(&ca)).await;
    let answer = send(by_config.addr, Method::GET, "/v1/models", &[], "").await;
    assert_eq!(answer.status, 200);
    by_config.stop().await;

    // A missing file is skipped; the other one is still used.
    let by_ssl = run(
        &url,
        &[
            ("HTTP_PROXY", &corp.url()),
            ("NODE_EXTRA_CA_CERTS", "/no/such/ca.pem"),
            ("SSL_CERT_FILE", &ca),
        ],
        None,
    )
    .await;
    let answer = send(by_ssl.addr, Method::GET, "/v1/models", &[], "").await;
    assert_eq!(answer.status, 200);
    by_ssl.stop().await;
}

#[tokio::test]
async fn an_unknown_authority_is_refused_with_a_bad_gateway() {
    let pki = Pki::new();
    let upstream = TlsUpstream::start(&pki).await;
    let corp = FakeProxy::start(Behavior::Tunnel, upstream.addr).await;
    let proxy = run(&upstream.named_url(), &[("HTTPS_PROXY", &corp.url())], None).await;

    let answer = send(proxy.addr, Method::GET, "/v1/models", &[], "").await;
    assert_eq!(answer.status, 502);
    assert_eq!(upstream.served(), 0);
    proxy.stop().await;
}

#[tokio::test]
async fn no_proxy_connects_directly() {
    let pki = Pki::new();
    let upstream = TlsUpstream::start(&pki).await;
    // Would refuse everything: it must not be asked.
    let corp = FakeProxy::start(Behavior::Refuse(403), upstream.addr).await;
    let (_dir, ca) = ca_file(&pki);

    let proxy = run(
        &upstream.direct_url(),
        &[
            ("HTTPS_PROXY", &corp.url()),
            ("NO_PROXY", "localhost,127.0.0.1"),
            ("SSL_CERT_FILE", &ca),
        ],
        None,
    )
    .await;
    let answer = send(proxy.addr, Method::GET, "/v1/models", &[], "").await;
    assert_eq!(answer.status, 200);
    assert!(corp.heads().is_empty());
    proxy.stop().await;
}

#[tokio::test]
async fn a_malformed_proxy_value_connects_directly() {
    let pki = Pki::new();
    let upstream = TlsUpstream::start(&pki).await;
    let (_dir, ca) = ca_file(&pki);

    for bad in ["ht tp://corp", "socks5://corp:1080"] {
        let proxy = run(
            &upstream.direct_url(),
            &[("HTTPS_PROXY", bad), ("SSL_CERT_FILE", &ca)],
            None,
        )
        .await;
        let answer = send(proxy.addr, Method::GET, "/v1/models", &[], "").await;
        assert_eq!(answer.status, 200, "{bad}");
        proxy.stop().await;
    }
}

#[tokio::test]
async fn a_proxy_that_refuses_connect_is_a_bad_gateway() {
    let pki = Pki::new();
    let upstream = TlsUpstream::start(&pki).await;
    let (_dir, ca) = ca_file(&pki);

    for status in [403, 407] {
        let corp = FakeProxy::start(Behavior::Refuse(status), upstream.addr).await;
        let proxy = run(
            &upstream.named_url(),
            &[("HTTPS_PROXY", &corp.url()), ("SSL_CERT_FILE", &ca)],
            None,
        )
        .await;
        let answer = send(proxy.addr, Method::GET, "/v1/models", &[], "").await;
        assert_eq!(answer.status, 502, "{status}");
        assert_eq!(
            answer.json()["error"]["message"],
            "tokred proxy: the upstream could not be reached"
        );
        assert_eq!(corp.heads().len(), 1);
        assert_eq!(upstream.served(), 0);
        let status = send(
            proxy.addr,
            Method::GET,
            "/__tokred/status",
            &[("x-tokred-secret", support::SECRET)],
            "",
        )
        .await;
        assert_eq!(status.json()["upstream_errors"], 1);
        proxy.stop().await;
    }
}

#[tokio::test]
async fn a_proxy_that_is_down_is_a_bad_gateway() {
    let pki = Pki::new();
    let upstream = TlsUpstream::start(&pki).await;
    let down = format!("http://{}", closed_port().await);
    let proxy = run(&upstream.named_url(), &[("HTTPS_PROXY", &down)], None).await;
    let answer = send(proxy.addr, Method::GET, "/v1/models", &[], "").await;
    assert_eq!(answer.status, 502);
    proxy.stop().await;
}
