//! The `/__tokred/*` endpoints (spec 6.8). They are answered by the proxy and never forwarded.
//! `health` is open; `status` and `shutdown` need the `x-tokred-secret` header.

use hyper::{HeaderMap, Method, StatusCode};

/// Prefix of every admin path.
pub(crate) const PREFIX: &str = "/__tokred/";

/// Header carrying the admin secret.
pub(crate) const SECRET_HEADER: &str = "x-tokred-secret";

/// What an admin request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admin {
    /// `GET /__tokred/health`.
    Health,
    /// `GET /__tokred/status` (secret).
    Status,
    /// `POST /__tokred/shutdown` (secret).
    Shutdown,
    /// `POST /__tokred/dashboard-token` (secret): a one-time token to open the dashboard.
    DashboardToken,
    /// A known path with the wrong method.
    WrongMethod,
    /// Unknown admin path.
    Unknown,
}

/// The admin action for a path, or `None` when the path is not an admin path.
pub(crate) fn route(method: &Method, path: &str) -> Option<Admin> {
    let name = path.strip_prefix(PREFIX)?;
    Some(match (name, method) {
        ("health", &Method::GET) => Admin::Health,
        ("status", &Method::GET) => Admin::Status,
        ("shutdown", &Method::POST) => Admin::Shutdown,
        ("dashboard-token", &Method::POST) => Admin::DashboardToken,
        ("health" | "status" | "shutdown" | "dashboard-token", _) => Admin::WrongMethod,
        _ => Admin::Unknown,
    })
}

/// Whether `given` equals `expected`, in time that does not depend on where they differ (only
/// on their lengths).
pub(crate) fn constant_time_eq(given: &[u8], expected: &[u8]) -> bool {
    let mut difference = u8::from(given.len() != expected.len());
    for (index, byte) in expected.iter().enumerate() {
        difference |= byte ^ given.get(index).copied().unwrap_or(0);
    }
    difference == 0
}

/// Compares the header with the configured secret in constant time. An empty configured secret
/// matches nothing.
pub(crate) fn authorized(headers: &HeaderMap, secret: &str) -> bool {
    let given = headers
        .get(SECRET_HEADER)
        .map_or(&b""[..], |value| value.as_bytes());
    !secret.is_empty() && constant_time_eq(given, secret.as_bytes())
}

/// Status code and JSON body for a refused admin request.
pub(crate) fn refusal(action: Admin) -> (StatusCode, &'static str) {
    match action {
        Admin::WrongMethod => (StatusCode::METHOD_NOT_ALLOWED, "method not allowed"),
        Admin::Unknown => (StatusCode::NOT_FOUND, "unknown admin endpoint"),
        Admin::Health | Admin::Status | Admin::Shutdown | Admin::DashboardToken => {
            (StatusCode::UNAUTHORIZED, "missing or wrong x-tokred-secret")
        }
    }
}

#[cfg(test)]
mod tests {
    use hyper::header::HeaderValue;

    use super::*;

    #[test]
    fn admin_paths_are_routed() {
        assert_eq!(route(&Method::GET, "/__tokred/health"), Some(Admin::Health));
        assert_eq!(route(&Method::GET, "/__tokred/status"), Some(Admin::Status));
        assert_eq!(
            route(&Method::POST, "/__tokred/shutdown"),
            Some(Admin::Shutdown)
        );
        assert_eq!(
            route(&Method::GET, "/__tokred/shutdown"),
            Some(Admin::WrongMethod)
        );
        assert_eq!(
            route(&Method::POST, "/__tokred/dashboard-token"),
            Some(Admin::DashboardToken)
        );
        assert_eq!(
            route(&Method::GET, "/__tokred/dashboard-token"),
            Some(Admin::WrongMethod)
        );
        assert_eq!(route(&Method::GET, "/__tokred/other"), Some(Admin::Unknown));
        assert_eq!(route(&Method::GET, "/v1/messages"), None);
        assert_eq!(route(&Method::GET, "/__tokred"), None);
    }

    #[test]
    fn the_secret_must_match_exactly() {
        let with = |value: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(SECRET_HEADER, HeaderValue::from_static(value));
            headers
        };
        assert!(authorized(&with("abc123"), "abc123"));
        assert!(!authorized(&with("abc124"), "abc123"));
        assert!(!authorized(&with("abc"), "abc123"));
        assert!(!authorized(&with("abc1234"), "abc123"));
        assert!(!authorized(&HeaderMap::new(), "abc123"));
        assert!(!authorized(&with(""), ""), "an empty secret disables admin");
    }

    #[test]
    fn the_comparison_is_exact_and_length_sensitive() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(constant_time_eq(b"", b""));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"abcd", b"abc"));
        assert!(!constant_time_eq(b"", b"a"));
    }

    #[test]
    fn refusals() {
        assert_eq!(
            refusal(Admin::WrongMethod).0,
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(refusal(Admin::Unknown).0, StatusCode::NOT_FOUND);
        assert_eq!(refusal(Admin::Status).0, StatusCode::UNAUTHORIZED);
    }
}
