//! Corporate-network harness: a throwaway certificate authority, a fake HTTPS upstream signed by
//! it, and a fake HTTP proxy that answers `CONNECT`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::Full;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};
use rustls::ServerConfig as TlsServerConfig;
use rustls::crypto::ring::default_provider;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokred_proxy::NetworkSettings;

/// The name the fake upstream's certificate is valid for besides `127.0.0.1`.
pub const UPSTREAM_NAME: &str = "upstream.test";

/// What the fake upstream answers to every request.
pub const UPSTREAM_BODY: &str = r#"{"served_by":"tls-upstream"}"#;

/// A certificate authority and a server certificate it signed.
pub struct Pki {
    pub ca_pem: String,
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

impl Pki {
    pub fn new() -> Self {
        let ca_key = KeyPair::generate().unwrap();
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "tokred test CA");
        let ca = ca_params.self_signed(&ca_key).unwrap();
        let issuer = Issuer::new(ca_params, ca_key);

        let leaf_key = KeyPair::generate().unwrap();
        let leaf = CertificateParams::new(vec![UPSTREAM_NAME.to_owned(), "127.0.0.1".to_owned()])
            .unwrap()
            .signed_by(&leaf_key, &issuer)
            .unwrap();
        Self {
            ca_pem: ca.pem(),
            chain: vec![leaf.der().clone()],
            key: PrivateKeyDer::try_from(leaf_key.serialize_der()).unwrap(),
        }
    }

    /// Writes the authority's PEM into `dir` and returns the path.
    pub fn write_ca(&self, dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, &self.ca_pem).unwrap();
        path
    }

    fn acceptor(&self) -> TlsAcceptor {
        let config = TlsServerConfig::builder_with_provider(Arc::new(default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(self.chain.clone(), self.key.clone_key())
            .unwrap();
        TlsAcceptor::from(Arc::new(config))
    }
}

/// A fake HTTPS API signed by a [`Pki`]; it counts the requests it served.
pub struct TlsUpstream {
    pub addr: SocketAddr,
    pub served: Arc<Mutex<Vec<String>>>,
}

impl TlsUpstream {
    pub async fn start(pki: &Pki) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served = Arc::new(Mutex::new(Vec::new()));
        let log = served.clone();
        let acceptor = pki.acceptor();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let acceptor = acceptor.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    // A client that does not trust the certificate fails the handshake here.
                    let Ok(tls) = acceptor.accept(stream).await else {
                        return;
                    };
                    let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                        let log = log.clone();
                        async move {
                            log.lock().unwrap().push(request.uri().to_string());
                            Ok::<_, Infallible>(Response::new(Full::new(Bytes::from(
                                UPSTREAM_BODY,
                            ))))
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(tls), service)
                        .await;
                });
            }
        });
        Self { addr, served }
    }

    /// `https://upstream.test:<port>`: the name does not resolve, so only a tunnel gets there.
    pub fn named_url(&self) -> String {
        format!("https://{UPSTREAM_NAME}:{}", self.addr.port())
    }

    /// `https://127.0.0.1:<port>`.
    pub fn direct_url(&self) -> String {
        format!("https://{}", self.addr)
    }

    pub fn served(&self) -> usize {
        self.served.lock().unwrap().len()
    }
}

/// What the fake corporate proxy does with a `CONNECT`.
#[derive(Debug, Clone, Copy)]
pub enum Behavior {
    /// Answer 200 and tunnel to the real upstream.
    Tunnel,
    /// Answer with this status instead.
    Refuse(u16),
}

/// A fake HTTP proxy. Every `CONNECT` it receives is recorded as its raw head.
pub struct FakeProxy {
    pub addr: SocketAddr,
    pub heads: Arc<Mutex<Vec<String>>>,
}

impl FakeProxy {
    pub async fn start(behavior: Behavior, upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let heads = Arc::new(Mutex::new(Vec::new()));
        let log = heads.clone();
        tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    return;
                };
                let log = log.clone();
                tokio::spawn(async move {
                    let _ = serve_connect(client, behavior, upstream, log).await;
                });
            }
        });
        Self { addr, heads }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn heads(&self) -> Vec<String> {
        self.heads.lock().unwrap().clone()
    }
}

async fn serve_connect(
    mut client: TcpStream,
    behavior: Behavior,
    upstream: SocketAddr,
    log: Arc<Mutex<Vec<String>>>,
) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        client.read_exact(&mut byte).await?;
        head.push(byte[0]);
    }
    log.lock()
        .unwrap()
        .push(String::from_utf8_lossy(&head).into_owned());
    match behavior {
        Behavior::Refuse(status) => {
            let reply = format!("HTTP/1.1 {status} Refused\r\ncontent-length: 0\r\n\r\n");
            client.write_all(reply.as_bytes()).await
        }
        Behavior::Tunnel => {
            let mut target = TcpStream::connect(upstream).await?;
            client
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await?;
            tokio::io::copy_bidirectional(&mut client, &mut target)
                .await
                .map(|_| ())
        }
    }
}

/// [`NetworkSettings`] from a list of variables, without the operating system's roots.
pub fn settings(vars: &[(&str, &str)], ca_bundle: Option<&str>) -> NetworkSettings {
    let map: HashMap<String, String> = vars
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    NetworkSettings::from_lookup(&|name| map.get(name).cloned(), ca_bundle).with_native_roots(false)
}
