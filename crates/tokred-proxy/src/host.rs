//! Address rules: which hosts count as loopback, which `Host` headers the proxy answers (the
//! DNS-rebinding defence of spec 6.1) and where it may listen.

use std::net::{IpAddr, SocketAddr};

use crate::error::ProxyError;

/// Splits a `Host` value into its name and optional port. `[::1]:1029` keeps the brackets out.
fn split_host(host: &str) -> Option<(&str, Option<&str>)> {
    if let Some(rest) = host.strip_prefix('[') {
        let (name, after) = rest.split_once(']')?;
        return match after {
            "" => Some((name, None)),
            _ => Some((name, Some(after.strip_prefix(':')?))),
        };
    }
    Some(match host.rsplit_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (host, None),
    })
}

/// `localhost` or a loopback IP literal (brackets around IPv6 allowed).
pub(crate) fn is_loopback_name(name: &str) -> bool {
    let bare = name.trim_start_matches('[').trim_end_matches(']');
    bare.eq_ignore_ascii_case("localhost")
        || bare.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Whether a request's `Host` header names this proxy: a loopback name (or, when listening on a
/// non-loopback address, that address) with the bound port. DNS names other than `localhost`
/// are refused, which is what defeats DNS rebinding.
pub(crate) fn host_allowed(host: Option<&str>, bound: SocketAddr) -> bool {
    let Some((name, port)) = host.and_then(split_host) else {
        return false;
    };
    let port_ok = match port {
        Some(port) => port.parse::<u16>().ok() == Some(bound.port()),
        None => bound.port() == 80,
    };
    let ip = name.parse::<IpAddr>().ok();
    let name_ok = is_loopback_name(name)
        || ip.is_some_and(|ip| ip == bound.ip() || bound.ip().is_unspecified());
    port_ok && name_ok
}

/// Refuses a non-loopback listening address unless it was explicitly allowed.
pub(crate) fn check_bind(addr: SocketAddr, allow_non_loopback: bool) -> Result<(), ProxyError> {
    if addr.ip().is_loopback() || allow_non_loopback {
        Ok(())
    } else {
        Err(ProxyError::NonLoopback(addr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn loopback_hosts_with_the_bound_port_are_allowed() {
        let bound = addr("127.0.0.1:1029");
        for host in [
            "127.0.0.1:1029",
            "localhost:1029",
            "LocalHost:1029",
            "[::1]:1029",
            "127.0.0.2:1029",
        ] {
            assert!(host_allowed(Some(host), bound), "{host}");
        }
    }

    #[test]
    fn other_hosts_and_ports_are_refused() {
        let bound = addr("127.0.0.1:1029");
        for host in [
            None,
            Some("evil.example:1029"),
            Some("127.0.0.1:9999"),
            Some("127.0.0.1"),
            Some("localhost:abc"),
            Some("[::1"),
            Some("[::1]x"),
            Some("10.0.0.1:1029"),
            Some(""),
        ] {
            assert!(!host_allowed(host, bound), "{host:?}");
        }
    }

    #[test]
    fn a_missing_port_means_port_80() {
        assert!(host_allowed(Some("localhost"), addr("127.0.0.1:80")));
        assert!(host_allowed(Some("[::1]"), addr("[::1]:80")));
    }

    #[test]
    fn a_non_loopback_bind_also_answers_its_own_address() {
        assert!(host_allowed(Some("10.0.0.1:1029"), addr("10.0.0.1:1029")));
        assert!(!host_allowed(Some("10.0.0.2:1029"), addr("10.0.0.1:1029")));
        assert!(host_allowed(Some("10.0.0.2:1029"), addr("0.0.0.0:1029")));
        assert!(
            !host_allowed(Some("rebind.example:1029"), addr("0.0.0.0:1029")),
            "DNS names stay refused"
        );
    }

    #[test]
    fn loopback_names() {
        assert!(is_loopback_name("localhost"));
        assert!(is_loopback_name("[::1]"));
        assert!(is_loopback_name("127.0.0.1"));
        assert!(!is_loopback_name("api.anthropic.com"));
        assert!(!is_loopback_name("10.1.1.1"));
    }

    #[test]
    fn binding_outside_loopback_needs_the_flag() {
        assert!(check_bind(addr("127.0.0.1:0"), false).is_ok());
        assert!(check_bind(addr("[::1]:0"), false).is_ok());
        assert!(matches!(
            check_bind(addr("0.0.0.0:1029"), false),
            Err(ProxyError::NonLoopback(_))
        ));
        assert!(check_bind(addr("0.0.0.0:1029"), true).is_ok());
    }
}
