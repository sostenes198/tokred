//! The configured upstream and the URI each request is forwarded to.

use hyper::Uri;

use crate::error::ProxyError;
use crate::host::is_loopback_name;

/// Where requests go: `https://<authority>[/base]`, or plain `http` for a loopback host (tests
/// and local gateways).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Upstream {
    /// `scheme://authority`.
    origin: String,
    /// Path prefix without a trailing slash (usually empty).
    base_path: String,
}

impl Upstream {
    /// Validates the configured URL.
    pub(crate) fn parse(raw: &str) -> Result<Self, ProxyError> {
        let invalid = |reason| ProxyError::InvalidUpstream {
            upstream: raw.to_owned(),
            reason,
        };
        let uri: Uri = raw.trim().parse().map_err(|_| invalid("not a URL"))?;
        let (Some(scheme), Some(authority)) = (uri.scheme_str(), uri.authority()) else {
            return Err(invalid(
                "must be an absolute URL such as https://api.anthropic.com",
            ));
        };
        match scheme {
            "https" => {}
            "http" if is_loopback_name(authority.host()) => {}
            "http" => {
                return Err(invalid(
                    "plain http is only allowed for a loopback upstream",
                ));
            }
            _ => return Err(invalid("the scheme must be https")),
        }
        Ok(Self {
            origin: format!("{scheme}://{authority}"),
            base_path: uri.path().trim_end_matches('/').to_owned(),
        })
    }

    /// The upstream URI for an incoming request URI (its path and query are kept as they are).
    pub(crate) fn target(&self, incoming: &Uri) -> Result<Uri, String> {
        let path = incoming.path_and_query().map_or("/", |pq| pq.as_str());
        let target = format!("{}{}{path}", self.origin, self.base_path);
        target
            .parse()
            .map_err(|_| format!("cannot build the upstream URI `{target}`"))
    }

    /// `scheme://authority/base`, for the status endpoint.
    pub(crate) fn display(&self) -> String {
        format!("{}{}", self.origin, self.base_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(upstream: &str, incoming: &str) -> String {
        Upstream::parse(upstream)
            .unwrap()
            .target(&incoming.parse().unwrap())
            .unwrap()
            .to_string()
    }

    #[test]
    fn paths_and_queries_are_appended_to_the_upstream() {
        assert_eq!(
            target("https://api.anthropic.com", "/v1/messages?beta=true"),
            "https://api.anthropic.com/v1/messages?beta=true"
        );
        assert_eq!(
            target("https://gw.example/anthropic/", "/v1/models"),
            "https://gw.example/anthropic/v1/models"
        );
        assert_eq!(
            target("http://127.0.0.1:9000", "http://other/v1/x?a=1"),
            "http://127.0.0.1:9000/v1/x?a=1"
        );
        assert_eq!(target("https://h", "h:443"), "https://h/");
    }

    #[test]
    fn only_https_or_loopback_http_is_accepted() {
        assert!(Upstream::parse("https://api.anthropic.com").is_ok());
        assert!(Upstream::parse("http://localhost:1").is_ok());
        assert!(Upstream::parse("http://[::1]:1").is_ok());
        for bad in [
            "http://api.anthropic.com",
            "ftp://127.0.0.1",
            "/relative",
            "not a url",
        ] {
            assert!(
                matches!(
                    Upstream::parse(bad),
                    Err(ProxyError::InvalidUpstream { .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_target_that_cannot_be_built_is_an_error() {
        let upstream = Upstream {
            origin: "https://h".to_owned(),
            base_path: " bad".to_owned(),
        };
        assert!(
            upstream
                .target(&"/x".parse().unwrap())
                .unwrap_err()
                .contains("https://h bad/x")
        );
        assert_eq!(upstream.display(), "https://h bad");
    }
}
