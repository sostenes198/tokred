//! The HTTP client of the proxy: TLS with the bundled Mozilla roots plus the operating system's
//! and the configured authorities, straight to the upstream or tunnelled through a corporate
//! HTTP proxy with `CONNECT`.

use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::{Request, Response, Uri};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::connect::proxy::Tunnel;
use hyper_util::client::legacy::{Client, Error};
use hyper_util::rt::{TokioExecutor, TokioTimer};
use rustls::crypto::ring::default_provider;
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject;

use crate::error::ProxyError;
use crate::network::{NetworkSettings, Route};
use crate::upstream::Upstream;

type Direct = Client<HttpsConnector<HttpConnector>, Full<Bytes>>;
type Tunnelled = Client<HttpsConnector<Tunnel<HttpConnector>>, Full<Bytes>>;

/// An HTTP/1 client for the upstream. No timeouts: the client's own timeout is the only one.
pub(crate) enum UpstreamClient {
    /// Connects to the upstream itself.
    Direct(Direct),
    /// Connects to a corporate proxy and asks it for a tunnel to the upstream.
    Tunnelled(Tunnelled),
}

impl std::fmt::Debug for UpstreamClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self {
            Self::Direct(_) => "Direct",
            Self::Tunnelled(_) => "Tunnelled",
        };
        f.debug_tuple("UpstreamClient").field(&kind).finish()
    }
}

impl UpstreamClient {
    /// The client for `upstream` under `network`. Problems with the network settings are logged
    /// once and never stop the proxy: it connects directly, or trusts fewer authorities.
    pub(crate) fn build(
        upstream: &Upstream,
        network: &NetworkSettings,
    ) -> Result<Self, ProxyError> {
        let origin = upstream.target(&Uri::from_static("/")).unwrap_or_default();
        let (route, warning) = network.route(&origin);
        if let Some(warning) = warning {
            tracing::warn!("{warning}");
        }
        let tls = tls_config(origin.scheme_str() == Some("https"), network)?;
        let builder = HttpsConnectorBuilder::new()
            .with_tls_config(tls)
            .https_or_http()
            .enable_http1();
        let executor = || {
            let mut client = Client::builder(TokioExecutor::new());
            client.pool_timer(TokioTimer::new());
            client
        };
        Ok(match route {
            Route::Direct => {
                let mut http = HttpConnector::new();
                // The HTTPS wrapper hands it `https` URIs.
                http.enforce_http(false);
                Self::Direct(executor().build(builder.wrap_connector(http)))
            }
            Route::Tunnel { proxy, auth } => {
                tracing::info!(proxy = %proxy.authority().map(ToString::to_string).unwrap_or_default(), "upstream requests go through an HTTP proxy");
                let tunnel = Tunnel::new(proxy, HttpConnector::new());
                let tunnel = auth.into_iter().fold(tunnel, Tunnel::with_auth);
                Self::Tunnelled(executor().build(builder.wrap_connector(tunnel)))
            }
        })
    }

    /// Sends one request.
    pub(crate) async fn request(
        &self,
        request: Request<Full<Bytes>>,
    ) -> Result<Response<Incoming>, Error> {
        match self {
            Self::Direct(client) => client.request(request).await,
            Self::Tunnelled(client) => client.request(request).await,
        }
    }
}

/// TLS settings that trust, for an `https` upstream, the bundled Mozilla roots, the operating
/// system's store (when enabled) and the extra PEM files. Nothing is loaded for plain `http`.
fn tls_config(https: bool, network: &NetworkSettings) -> Result<ClientConfig, ProxyError> {
    let mut roots = RootCertStore::empty();
    if https {
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        if network.native_roots() {
            add_native_roots(&mut roots);
        }
        network
            .ca_files()
            .iter()
            .for_each(|path| add_pem_file(&mut roots, path));
    }
    Ok(
        ClientConfig::builder_with_provider(Arc::new(default_provider()))
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

fn add_native_roots(roots: &mut RootCertStore) {
    let native = rustls_native_certs::load_native_certs();
    native.errors.iter().for_each(warn_native);
    roots.add_parsable_certificates(native.certs);
}

fn warn_native(error: &rustls_native_certs::Error) {
    tracing::warn!(%error, "could not load a certificate of the operating system store");
}

/// Adds the certificates of a PEM bundle; a missing or unusable file is logged and skipped.
fn add_pem_file(roots: &mut RootCertStore, path: &Path) {
    match read_pem(path) {
        Ok(certs) => {
            let (added, _ignored) = roots.add_parsable_certificates(certs);
            if added == 0 {
                tracing::warn!(file = %path.display(), "the CA file has no usable certificate; ignoring it");
            }
        }
        Err(reason) => {
            tracing::warn!(file = %path.display(), %reason, "could not use the CA file; ignoring it");
        }
    }
}

fn read_pem(path: &Path) -> Result<Vec<CertificateDer<'static>>, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    CertificateDer::pem_slice_iter(&bytes)
        .collect::<Result<_, _>>()
        .map_err(|error| format!("not a PEM bundle: {error}"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn https() -> Upstream {
        Upstream::parse("https://api.anthropic.com").unwrap()
    }

    #[test]
    fn a_pem_bundle_is_read_and_bad_files_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let garbage = dir.path().join("garbage.pem");
        std::fs::write(
            &garbage,
            "-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        let empty = dir.path().join("empty.pem");
        std::fs::write(&empty, "nothing here").unwrap();
        let broken_der = dir.path().join("broken.pem");
        std::fs::write(
            &broken_der,
            "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
        )
        .unwrap();

        assert!(read_pem(&dir.path().join("missing.pem")).is_err());
        assert!(read_pem(&garbage).unwrap_err().contains("not a PEM bundle"));
        assert!(read_pem(&empty).unwrap().is_empty());

        let mut roots = RootCertStore::empty();
        for path in [
            &garbage,
            &empty,
            &broken_der,
            &dir.path().join("missing.pem"),
        ] {
            add_pem_file(&mut roots, path);
        }
        assert!(roots.is_empty());
    }

    #[test]
    fn native_roots_are_loaded_and_their_errors_are_logged() {
        let mut roots = RootCertStore::empty();
        add_native_roots(&mut roots);
        warn_native(&rustls_native_certs::Error {
            context: "test",
            kind: rustls_native_certs::ErrorKind::Os("boom".into()),
        });
    }

    #[test]
    fn roots_are_only_loaded_for_an_https_upstream() {
        let none = NetworkSettings::default();
        assert!(tls_config(true, &none).is_ok());
        let http = Upstream::parse("http://127.0.0.1:1").unwrap();
        assert!(UpstreamClient::build(&http, &none).is_ok());
    }

    #[test]
    fn the_client_is_direct_unless_a_proxy_applies() {
        let direct = UpstreamClient::build(&https(), &NetworkSettings::default()).unwrap();
        assert!(format!("{direct:?}").contains("Direct"));
        let vars = |name: &str| (name == "HTTPS_PROXY").then(|| "http://u:pw@p:3128".to_owned());
        let settings = NetworkSettings::from_lookup(&vars, None).with_native_roots(false);
        let tunnelled = UpstreamClient::build(&https(), &settings).unwrap();
        assert!(format!("{tunnelled:?}").contains("Tunnelled"));
        let bad = |name: &str| (name == "HTTPS_PROXY").then(|| "socks5://p:1".to_owned());
        let settings =
            NetworkSettings::from_lookup(&bad, Some("/no/such.pem")).with_native_roots(false);
        let fallback = UpstreamClient::build(&https(), &settings).unwrap();
        assert!(format!("{fallback:?}").contains("Direct"));
        let _ = PathBuf::new();
    }
}
