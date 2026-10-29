//! Errors of the proxy's lifecycle (binding and serving). Per-request failures never surface
//! here: they become a response to the client (fail-open).

use std::net::SocketAddr;

/// The proxy could not start or keep serving.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// The bind address is not loopback and `allow_non_loopback` was not set.
    #[error("refusing to listen on {0}: the proxy only listens on loopback addresses")]
    NonLoopback(SocketAddr),
    /// The bind address could not be parsed.
    #[error("invalid bind address `{0}`")]
    InvalidBind(String),
    /// The upstream URL is not usable.
    #[error("invalid upstream `{upstream}`: {reason}")]
    InvalidUpstream {
        /// The configured value.
        upstream: String,
        /// Why it was refused.
        reason: &'static str,
    },
    /// The TLS client could not be set up.
    #[error("could not set up TLS for the upstream: {0}")]
    Tls(#[from] rustls::Error),
    /// The listening socket could not be opened.
    #[error("could not listen on {addr}: {source}")]
    Bind {
        /// The address asked for.
        addr: SocketAddr,
        /// The operating system's error.
        #[source]
        source: std::io::Error,
    },
    /// Any other I/O failure (socket setup, runtime start).
    #[error("proxy I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_readable() {
        let addr: SocketAddr = "10.0.0.1:1029".parse().unwrap();
        assert!(
            ProxyError::NonLoopback(addr)
                .to_string()
                .contains("10.0.0.1:1029")
        );
        assert!(
            ProxyError::InvalidBind("x".to_owned())
                .to_string()
                .contains("`x`")
        );
        let upstream = ProxyError::InvalidUpstream {
            upstream: "ftp://x".to_owned(),
            reason: "nope",
        };
        assert_eq!(upstream.to_string(), "invalid upstream `ftp://x`: nope");
        let bind = ProxyError::Bind {
            addr,
            source: std::io::Error::other("busy"),
        };
        assert!(bind.to_string().contains("busy"));
        assert!(std::error::Error::source(&bind).is_some());
        let tls = ProxyError::from(rustls::Error::General("no provider".to_owned()));
        assert!(tls.to_string().contains("no provider"));
        assert!(
            ProxyError::from(std::io::Error::other("io"))
                .to_string()
                .contains("io")
        );
    }
}
