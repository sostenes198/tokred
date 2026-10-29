//! Header handling. Only hop-by-hop headers are removed; everything else, including unknown
//! `anthropic-*` and `x-claude-code-*` headers and credentials, passes unchanged (the
//! gateway guide's "forward as open lists"). Header values are never logged.
//!
//! The one header that is rewritten is `Accept-Encoding`: the real API compresses its responses
//! when the client offers `gzip` (Claude Code does), and the proxy reads usage from the response,
//! so it asks the upstream for an uncompressed one. Only loopback bytes are added.

use hyper::HeaderMap;
use hyper::header::{ACCEPT_ENCODING, CONNECTION, CONTENT_LENGTH, HOST, HeaderName, HeaderValue};

/// Hop-by-hop headers (RFC 9110 section 7.6.1, plus the legacy `keep-alive`/`proxy-connection`).
const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// Names listed in `Connection` (they are hop-by-hop for this message too).
fn connection_listed(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect()
}

fn without(headers: &HeaderMap, extra: &[HeaderName]) -> HeaderMap {
    let listed = connection_listed(headers);
    let mut out = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let lower = name.as_str();
        let drop = HOP_BY_HOP.contains(&lower)
            || extra.contains(name)
            || listed.iter().any(|listed| listed == lower);
        if !drop {
            out.append(name.clone(), value.clone());
        }
    }
    out
}

/// Headers to send upstream: the client's, minus hop-by-hop ones, `Host` (the client sets the
/// upstream's), `Content-Length` (recomputed from the body actually sent) and tokred's own
/// admin secret, which must never leave the machine. `Accept-Encoding` becomes `identity` (see
/// the module docs).
pub(crate) fn upstream_request(headers: &HeaderMap) -> HeaderMap {
    let secret = HeaderName::from_static(crate::admin::SECRET_HEADER);
    let mut out = without(headers, &[HOST, CONTENT_LENGTH, secret]);
    if out.contains_key(ACCEPT_ENCODING) {
        out.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    }
    out
}

/// Headers to return to the client: the upstream's, minus hop-by-hop ones. `Content-Length`
/// stays because the body is forwarded byte for byte.
pub(crate) fn client_response(headers: &HeaderMap) -> HeaderMap {
    without(headers, &[])
}

/// The first value of a header as text.
pub(crate) fn text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

#[cfg(test)]
mod tests {
    use hyper::header::HeaderValue;

    use super::*;

    fn map(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.append(*name, HeaderValue::from_static(value));
        }
        headers
    }

    #[test]
    fn request_headers_lose_hop_by_hop_host_length_and_the_admin_secret() {
        let headers = map(&[
            ("host", "127.0.0.1:1029"),
            ("content-length", "10"),
            ("connection", "keep-alive, X-Private-Hop"),
            ("keep-alive", "timeout=5"),
            ("transfer-encoding", "chunked"),
            ("x-private-hop", "1"),
            ("x-tokred-secret", "admin"),
            ("authorization", "Bearer secret"),
            ("x-api-key", "sk-secret"),
            ("anthropic-beta", "a,b"),
            ("anthropic-future-thing", "1"),
            ("x-claude-code-session-id", "s"),
            ("accept-encoding", "gzip"),
            ("x-multi", "1"),
            ("x-multi", "2"),
        ]);
        let out = upstream_request(&headers);
        let names: Vec<&str> = out.keys().map(HeaderName::as_str).collect();
        assert_eq!(
            names,
            [
                "authorization",
                "x-api-key",
                "anthropic-beta",
                "anthropic-future-thing",
                "x-claude-code-session-id",
                "accept-encoding",
                "x-multi",
            ]
        );
        assert_eq!(out.get_all("x-multi").iter().count(), 2);
        assert_eq!(out.get("accept-encoding").unwrap(), "identity");
        // A client that sent none does not get one added.
        assert!(!upstream_request(&map(&[("x-api-key", "k")])).contains_key("accept-encoding"));
    }

    #[test]
    fn response_headers_keep_length_and_rate_limits() {
        let headers = map(&[
            ("content-length", "5"),
            ("transfer-encoding", "chunked"),
            ("retry-after", "30"),
            ("anthropic-ratelimit-unified-status", "rejected"),
            ("connection", "close"),
        ]);
        let out = client_response(&headers);
        assert_eq!(text(&out, "content-length"), Some("5"));
        assert_eq!(text(&out, "retry-after"), Some("30"));
        assert!(out.get("transfer-encoding").is_none());
        assert!(out.get("connection").is_none());
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn text_reads_visible_values_only() {
        let mut headers = map(&[("request-id", "req_1")]);
        headers.insert("x-bin", HeaderValue::from_bytes(b"\xff").unwrap());
        assert_eq!(text(&headers, "request-id"), Some("req_1"));
        assert_eq!(text(&headers, "x-bin"), None);
        assert_eq!(text(&headers, "missing"), None);
    }
}
