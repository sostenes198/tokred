//! The credentials of a proxy address never reach the logs, and a bad network setting is
//! reported once by name. This file installs a global log subscriber, so it holds a single test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::io::Write;
use std::sync::{Arc, Mutex};

use hyper::Method;
use support::net::{Behavior, FakeProxy, Pki, TlsUpstream, settings};
use support::{config, send, start_with};
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

#[tokio::test]
async fn proxy_credentials_never_reach_the_logs() {
    let logs = Capture::default();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .finish(),
    )
    .unwrap();

    let pki = Pki::new();
    let upstream = TlsUpstream::start(&pki).await;
    let dir = tempfile::tempdir().unwrap();
    let ca = pki.write_ca(dir.path(), "ca.pem");
    let ca = ca.to_str().unwrap();
    let refusing = FakeProxy::start(Behavior::Refuse(407), upstream.addr).await;
    let tunnelling = FakeProxy::start(Behavior::Tunnel, upstream.addr).await;
    let with_login = |url: &str| url.replace("http://", "http://alice:hunter2@");

    // A working tunnel, a refusing proxy, an address that is not one, and a missing CA file.
    let cases: [(&str, String, u16); 3] = [
        (&upstream.named_url(), with_login(&tunnelling.url()), 200),
        (&upstream.named_url(), with_login(&refusing.url()), 502),
        (&upstream.direct_url(), "alice:hunter2@@@".to_owned(), 200),
    ];
    for (url, proxy_value, expected) in cases {
        let mut config = config(url);
        config.network = settings(
            &[
                ("HTTPS_PROXY", &proxy_value),
                ("NODE_EXTRA_CA_CERTS", ca),
                ("SSL_CERT_FILE", "/no/such/bundle.pem"),
            ],
            None,
        );
        let debug = format!("{config:?}");
        assert!(!debug.contains("hunter2") && !debug.contains("alice"));
        let proxy = start_with(config, Vec::new()).await;
        let answer = send(proxy.addr, Method::GET, "/v1/models", &[], "").await;
        assert_eq!(answer.status, expected);
        proxy.stop().await;
    }

    let text = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(!text.contains("hunter2"), "{text}");
    assert!(!text.contains("alice"), "{text}");
    assert!(!text.contains("YWxpY2U6aHVudGVyMg"), "{text}");
    assert!(
        text.contains("HTTPS_PROXY is not a valid proxy address"),
        "{text}"
    );
    assert!(text.contains("could not use the CA file"), "{text}");
    assert!(
        text.contains("upstream requests go through an HTTP proxy"),
        "{text}"
    );
    assert!(text.contains("the upstream request failed"), "{text}");
}
