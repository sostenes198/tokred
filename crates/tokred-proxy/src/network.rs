//! What the proxy needs to work behind a corporate network: the HTTP proxy to tunnel through
//! (`HTTPS_PROXY`, `NO_PROXY`) and the extra certificate authorities to trust
//! (`NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`, `[proxy] ca_bundle`). This module only decides; the
//! connector in `connector.rs` acts on the decision.
//!
//! A proxy address can carry credentials (`http://user:pass@host:3128`): they are kept in a
//! type that never prints them, and every message here names the variable, never its value.

use std::fmt;
use std::path::{Path, PathBuf};

use hyper::Uri;
use hyper::header::HeaderValue;
use hyper_util::client::proxy::matcher::Matcher;
use serde::Serialize;

/// A value that must not reach a log: `Debug` shows a placeholder.
#[derive(Clone, PartialEq, Eq)]
struct Redacted(String);

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// The proxy variable in effect, with the name it was found under.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProxyVar {
    name: String,
    value: Redacted,
}

/// Corporate-network settings of the proxy.
///
/// [`Default`] means "nothing special": no HTTP proxy, no extra authority and only the bundled
/// roots. [`NetworkSettings::from_env`] reads the environment the way Claude Code itself does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkSettings {
    proxy: Option<ProxyVar>,
    no_proxy: String,
    ca_files: Vec<PathBuf>,
    native_roots: bool,
}

/// How a request to the upstream gets there.
#[derive(Debug, Clone)]
pub(crate) enum Route {
    /// Straight to the upstream.
    Direct,
    /// Through an HTTP proxy with `CONNECT`.
    Tunnel {
        /// The proxy, without credentials.
        proxy: Uri,
        /// Its `Proxy-Authorization` value, when the address carried credentials.
        auth: Option<HeaderValue>,
    },
}

/// What the settings mean for one upstream.
enum Plan {
    /// No proxy variable applies (none set, or a plain-`http` loopback upstream).
    Unset,
    /// `NO_PROXY` excludes the upstream; the address is the proxy that would have been used.
    Bypassed(Uri),
    /// The variable is not an address.
    Malformed,
    /// A proxy kind that is not supported (`socks5`, `https`); the address and scheme.
    Unsupported(Uri, String),
    /// Tunnel through this proxy.
    Tunnel(Route, Uri),
}

/// The first non-empty value of `NAME` or `name`.
fn var(lookup: &dyn Fn(&str) -> Option<String>, name: &str) -> Option<(String, String)> {
    [name.to_owned(), name.to_lowercase()]
        .into_iter()
        .find_map(|key| {
            lookup(&key)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .map(|value| (key, value))
        })
}

impl NetworkSettings {
    /// Reads the process environment; `ca_bundle` is `[proxy] ca_bundle` from `tokred.toml`.
    #[must_use]
    pub fn from_env(ca_bundle: Option<&str>) -> Self {
        Self::from_lookup(&|name| std::env::var(name).ok(), ca_bundle)
    }

    /// Like [`NetworkSettings::from_env`] with the variables given by `lookup`.
    ///
    /// For an `https` upstream the proxy is `HTTPS_PROXY`, else `ALL_PROXY`, else `HTTP_PROXY`
    /// (many corporate images only set the last one); each name is also tried in lower case.
    #[must_use]
    pub fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>, ca_bundle: Option<&str>) -> Self {
        let proxy = ["HTTPS_PROXY", "ALL_PROXY", "HTTP_PROXY"]
            .iter()
            .find_map(|name| var(lookup, name))
            .map(|(name, value)| ProxyVar {
                name,
                value: Redacted(value),
            });
        let mut ca_files: Vec<PathBuf> = Vec::new();
        let listed = ca_bundle
            .map(|path| path.trim().to_owned())
            .into_iter()
            .chain(var(lookup, "NODE_EXTRA_CA_CERTS").map(|(_, path)| path))
            .chain(var(lookup, "SSL_CERT_FILE").map(|(_, path)| path));
        for path in listed.filter(|path| !path.is_empty()).map(PathBuf::from) {
            if !ca_files.contains(&path) {
                ca_files.push(path);
            }
        }
        Self {
            proxy,
            no_proxy: var(lookup, "NO_PROXY")
                .map(|(_, list)| list)
                .unwrap_or_default(),
            ca_files,
            native_roots: true,
        }
    }

    /// Whether the operating system's certificate store is trusted too (on by default when the
    /// settings come from the environment, off for [`Default`]).
    #[must_use]
    pub fn with_native_roots(mut self, native_roots: bool) -> Self {
        self.native_roots = native_roots;
        self
    }

    /// The PEM files trusted in addition to the bundled roots, in the order they are loaded.
    #[must_use]
    pub fn ca_files(&self) -> &[PathBuf] {
        &self.ca_files
    }

    /// Whether the operating system's certificate store is trusted.
    #[must_use]
    pub fn native_roots(&self) -> bool {
        self.native_roots
    }

    /// What the settings mean for `dst`, the upstream's origin.
    fn plan(&self, dst: &Uri) -> Plan {
        let Some(var) = self
            .proxy
            .as_ref()
            .filter(|_| dst.scheme_str() == Some("https"))
        else {
            return Plan::Unset;
        };
        let value = var.value.0.as_str();
        let matching = |no: &str| {
            Matcher::builder()
                .https(value)
                .no(no)
                .build()
                .intercept(dst)
        };
        let Some(found) = matching("") else {
            return Plan::Malformed;
        };
        let proxy = found.uri().clone();
        if matching(&self.no_proxy).is_none() {
            return Plan::Bypassed(proxy);
        }
        let scheme = proxy.scheme_str().unwrap_or_default().to_owned();
        if scheme == "http" {
            let route = Route::Tunnel {
                proxy: proxy.clone(),
                auth: found.basic_auth().cloned(),
            };
            return Plan::Tunnel(route, proxy);
        }
        Plan::Unsupported(proxy, scheme)
    }

    /// The route to `dst` and, when the settings cannot be honoured, why (never the value of
    /// the variable: it can hold a password).
    pub(crate) fn route(&self, dst: &Uri) -> (Route, Option<String>) {
        let name = self.proxy.as_ref().map_or("", |var| var.name.as_str());
        match self.plan(dst) {
            Plan::Tunnel(route, _) => (route, None),
            Plan::Malformed => (
                Route::Direct,
                Some(format!(
                    "{name} is not a valid proxy address (use http://host:port); connecting directly"
                )),
            ),
            Plan::Unsupported(_, scheme) => (
                Route::Direct,
                Some(format!(
                    "{name} uses {scheme}, which tokred does not support (only http:// proxies); connecting directly"
                )),
            ),
            Plan::Unset | Plan::Bypassed(_) => (Route::Direct, None),
        }
    }

    /// A description for `tokred doctor` and `tokred proxy status` (host names only; the
    /// credentials of the proxy address are left out). `upstream` is the configured upstream.
    #[must_use]
    pub fn summary(&self, upstream: &str) -> NetworkSummary {
        let dst = upstream.parse::<Uri>().unwrap_or_default();
        let name = self.proxy.as_ref().map_or("", |var| var.name.as_str());
        let host = |uri: &Uri| uri.authority().map(ToString::to_string);
        let (status, proxy_host) = match self.plan(&dst) {
            Plan::Unset => ("none", None),
            Plan::Malformed => ("invalid", Some("?".to_owned())),
            Plan::Bypassed(uri) => ("bypassed", host(&uri)),
            Plan::Unsupported(uri, _) => ("unsupported", host(&uri)),
            Plan::Tunnel(_, uri) => ("tunnel", host(&uri)),
        };
        NetworkSummary {
            proxy: proxy_host.map(|host| ProxySummary {
                variable: name.to_owned(),
                host,
                status: status.to_owned(),
            }),
            ca_files: self
                .ca_files
                .iter()
                .map(|path| CaFileSummary {
                    path: path.display().to_string(),
                    readable: readable(path),
                })
                .collect(),
        }
    }
}

fn readable(path: &Path) -> bool {
    std::fs::File::open(path).is_ok()
}

/// The proxy tokred tunnels through, as shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProxySummary {
    /// The variable it came from (`HTTPS_PROXY`, `https_proxy`, ...).
    pub variable: String,
    /// `host:port` of the proxy, without credentials; `?` when the value is not an address.
    pub host: String,
    /// `tunnel` (used), `bypassed` (`NO_PROXY` matches the upstream), `invalid` or
    /// `unsupported` (not an `http://` proxy); in the last two tokred connects directly.
    pub status: String,
}

/// An extra certificate authority file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaFileSummary {
    /// The path as configured.
    pub path: String,
    /// Whether the file can be opened.
    pub readable: bool,
}

/// The corporate-network settings in effect, for `tokred doctor` and `tokred proxy status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NetworkSummary {
    /// The HTTP proxy for the upstream, when a proxy variable is set.
    pub proxy: Option<ProxySummary>,
    /// The extra certificate authority files.
    pub ca_files: Vec<CaFileSummary>,
}

impl NetworkSummary {
    /// Human-readable lines (empty when nothing is configured).
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let proxy = self.proxy.iter().map(|proxy| {
            let meaning = match proxy.status.as_str() {
                "tunnel" => "the upstream is reached through it with CONNECT",
                "bypassed" => "not used: NO_PROXY matches the upstream",
                "unsupported" => "not supported (only http:// proxies); connecting directly",
                _ => "not a valid proxy address; connecting directly",
            };
            format!("{} {} ({meaning})", proxy.variable, proxy.host)
        });
        let files = self.ca_files.iter().map(|file| {
            let state = if file.readable {
                ""
            } else {
                " (cannot be read)"
            };
            format!("extra CA file {}{state}", file.path)
        });
        proxy.chain(files).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn settings(vars: &[(&str, &str)], ca_bundle: Option<&str>) -> NetworkSettings {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        NetworkSettings::from_lookup(&|name| map.get(name).cloned(), ca_bundle)
    }

    fn api() -> Uri {
        Uri::from_static("https://api.anthropic.com")
    }

    fn route(vars: &[(&str, &str)]) -> (Route, Option<String>) {
        settings(vars, None).route(&api())
    }

    #[test]
    fn nothing_set_means_a_direct_connection_and_native_roots() {
        let none = settings(&[], None);
        assert_eq!(none.proxy, None);
        assert!(none.ca_files().is_empty());
        assert!(none.native_roots());
        assert!(matches!(none.route(&api()), (Route::Direct, None)));
        assert_eq!(
            none.summary("https://api.anthropic.com"),
            NetworkSummary::default()
        );
        assert!(!NetworkSettings::default().native_roots());
        assert!(!none.with_native_roots(false).native_roots());
    }

    #[test]
    fn the_environment_is_read_through_the_lookup() {
        // The real environment: only check that it can be read.
        let _ = NetworkSettings::from_env(None);
    }

    #[test]
    fn an_http_proxy_becomes_a_tunnel_without_its_credentials() {
        let (route, warning) = route(&[("HTTPS_PROXY", "http://alice:s3cret@proxy.corp:3128")]);
        assert_eq!(warning, None);
        let Route::Tunnel { proxy, auth } = route else {
            panic!("expected a tunnel");
        };
        assert_eq!(proxy.to_string(), "http://proxy.corp:3128/");
        let auth = auth.unwrap();
        assert!(auth.is_sensitive());
        assert_eq!(auth.to_str().unwrap(), "Basic YWxpY2U6czNjcmV0");
    }

    #[test]
    fn a_proxy_without_a_scheme_is_http_and_without_credentials_has_no_auth() {
        let (route, _) = route(&[("https_proxy", "proxy.corp:3128")]);
        let Route::Tunnel { proxy, auth } = route else {
            panic!("expected a tunnel");
        };
        assert_eq!(proxy.to_string(), "http://proxy.corp:3128/");
        assert!(auth.is_none());
    }

    #[test]
    fn the_variables_have_a_priority_and_upper_case_wins() {
        let via = |vars: &[(&str, &str)]| {
            let (route, _) = route(vars);
            match route {
                Route::Tunnel { proxy, .. } => proxy.authority().unwrap().to_string(),
                Route::Direct => "direct".to_owned(),
            }
        };
        assert_eq!(via(&[("HTTP_PROXY", "http://c:1")]), "c:1");
        assert_eq!(
            via(&[("HTTP_PROXY", "http://c:1"), ("ALL_PROXY", "http://b:1")]),
            "b:1"
        );
        assert_eq!(
            via(&[
                ("HTTP_PROXY", "http://c:1"),
                ("ALL_PROXY", "http://b:1"),
                ("https_proxy", "http://a:1"),
                ("HTTPS_PROXY", "http://upper:1"),
            ]),
            "upper:1"
        );
        assert_eq!(via(&[("HTTPS_PROXY", "   ")]), "direct");
    }

    #[test]
    fn no_proxy_excludes_the_upstream() {
        for no_proxy in [
            "api.anthropic.com",
            ".anthropic.com",
            "internal, anthropic.com",
            "*",
        ] {
            let vars = [("HTTPS_PROXY", "http://p:1"), ("no_proxy", no_proxy)];
            let (route, warning) = route(&vars);
            assert!(matches!(route, Route::Direct), "{no_proxy}");
            assert_eq!(warning, None);
            let summary = settings(&vars, None).summary("https://api.anthropic.com");
            assert_eq!(summary.proxy.unwrap().status, "bypassed");
        }
        let (route, _) = self::route(&[("HTTPS_PROXY", "http://p:1"), ("NO_PROXY", "other.com")]);
        assert!(matches!(route, Route::Tunnel { .. }));
    }

    #[test]
    fn a_plain_http_upstream_never_uses_the_proxy() {
        let settings = settings(&[("HTTPS_PROXY", "http://p:1")], None);
        let (route, warning) = settings.route(&Uri::from_static("http://127.0.0.1:9"));
        assert!(matches!(route, Route::Direct));
        assert_eq!(warning, None);
    }

    #[test]
    fn a_malformed_value_is_reported_without_echoing_it() {
        for bad in [
            "ht tp://x",
            "http://pw@[bad",
            "pw@@@",
            "http://",
            "ftp://pw@host:1",
        ] {
            let (route, warning) = route(&[("HTTPS_PROXY", bad)]);
            assert!(matches!(route, Route::Direct), "{bad}");
            let warning = warning.unwrap();
            assert!(warning.contains("HTTPS_PROXY is not a valid proxy address"));
            assert!(!warning.contains("pw"));
        }
    }

    #[test]
    fn socks_and_https_proxies_are_reported_as_unsupported() {
        for (value, scheme) in [
            ("socks5://u:pw@proxy:1080", "socks5"),
            ("socks5h://proxy:1080", "socks5h"),
            ("https://u:pw@proxy:443", "https"),
        ] {
            let vars = [("ALL_PROXY", value)];
            let (route, warning) = route(&vars);
            assert!(matches!(route, Route::Direct));
            let warning = warning.unwrap();
            assert!(warning.contains(&format!("ALL_PROXY uses {scheme}")));
            assert!(!warning.contains("pw"));
            let summary = settings(&vars, None).summary("https://api.anthropic.com");
            let proxy = summary.proxy.unwrap();
            assert_eq!(proxy.status, "unsupported");
            assert!(!proxy.host.contains("pw"));
        }
    }

    #[test]
    fn debug_output_never_shows_the_proxy_address() {
        let settings = settings(
            &[("HTTPS_PROXY", "http://alice:s3cret@proxy.corp:3128")],
            None,
        );
        let text = format!("{settings:?}");
        assert!(text.contains("<redacted>") && text.contains("HTTPS_PROXY"));
        assert!(
            !text.contains("s3cret") && !text.contains("alice") && !text.contains("proxy.corp")
        );
    }

    #[test]
    fn extra_ca_files_come_from_the_config_and_both_variables_without_repeats() {
        let all = settings(
            &[
                ("NODE_EXTRA_CA_CERTS", "/etc/corp.pem"),
                ("SSL_CERT_FILE", "/etc/corp.pem"),
            ],
            Some(" /etc/own.pem "),
        );
        assert_eq!(
            all.ca_files(),
            [
                PathBuf::from("/etc/own.pem"),
                PathBuf::from("/etc/corp.pem")
            ]
        );
        let ssl = settings(&[("SSL_CERT_FILE", "/etc/ssl.pem")], Some(""));
        assert_eq!(ssl.ca_files(), [PathBuf::from("/etc/ssl.pem")]);
    }

    #[test]
    fn the_summary_lists_what_is_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("corp.pem");
        std::fs::write(&present, "x").unwrap();
        let missing = dir.path().join("missing.pem");
        let vars = [
            ("HTTPS_PROXY", "http://alice:s3cret@proxy.corp:3128"),
            ("NODE_EXTRA_CA_CERTS", present.to_str().unwrap()),
            ("SSL_CERT_FILE", missing.to_str().unwrap()),
        ];
        let summary = settings(&vars, None).summary("https://api.anthropic.com");
        let proxy = summary.proxy.clone().unwrap();
        assert_eq!(
            (
                proxy.variable.as_str(),
                proxy.host.as_str(),
                proxy.status.as_str()
            ),
            ("HTTPS_PROXY", "proxy.corp:3128", "tunnel")
        );
        assert_eq!(summary.ca_files.len(), 2);
        assert!(summary.ca_files[0].readable && !summary.ca_files[1].readable);
        let lines = summary.lines();
        assert_eq!(
            lines[0],
            "HTTPS_PROXY proxy.corp:3128 (the upstream is reached through it with CONNECT)"
        );
        assert_eq!(lines[1], format!("extra CA file {}", present.display()));
        assert!(lines[2].ends_with("(cannot be read)"));
        let text = serde_json::to_string(&summary).unwrap();
        assert!(!text.contains("s3cret") && !text.contains("alice"));
    }

    #[test]
    fn every_status_has_a_line() {
        let lines = |vars: &[(&str, &str)]| {
            settings(vars, None)
                .summary("https://api.anthropic.com")
                .lines()
                .join("\n")
        };
        assert!(lines(&[("HTTPS_PROXY", "x y z")]).contains("HTTPS_PROXY ? (not a valid"));
        assert!(lines(&[("HTTPS_PROXY", "socks5://s:1")]).contains("s:1 (not supported"));
        assert!(
            lines(&[("HTTPS_PROXY", "http://p:1"), ("NO_PROXY", "*")]).contains("p:1 (not used")
        );
        // An upstream that is not a URL is summarised as no proxy at all.
        let odd = settings(&[("HTTPS_PROXY", "http://p:1")], None).summary("not a url");
        assert_eq!(odd, NetworkSummary::default());
    }
}
