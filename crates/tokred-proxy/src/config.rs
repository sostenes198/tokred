//! Runtime settings of the proxy server.

use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokred_core::Timestamp;
use tokred_core::config::ProxyConfig;
use tokred_core::ports::Clock;

use crate::error::ProxyError;
use crate::network::NetworkSettings;

/// Sessions whose last main-loop request is remembered for the cache-miss explanation.
pub const DEFAULT_SESSION_CAP: usize = 1024;

/// How long a shutdown waits for responses in flight before giving up on them.
pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

/// The wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Timestamp(i64::try_from(secs).unwrap_or(i64::MAX))
    }
}

/// Everything the server needs to run. `Debug` never shows the admin secret.
#[derive(Clone)]
pub struct ServerConfig {
    /// Address to listen on (port 0 picks a free port; see [`crate::Proxy::local_addr`]).
    pub bind: SocketAddr,
    /// Allow a non-loopback `bind` (the `--bind` escape hatch). Off by default.
    pub allow_non_loopback: bool,
    /// The only host requests are forwarded to: `https://...`, or `http://` on loopback.
    pub upstream: String,
    /// Value the `x-tokred-secret` header must carry for `/__tokred/status` and
    /// `/__tokred/shutdown`. An empty secret disables both.
    pub admin_secret: String,
    /// Sessions remembered for the cache-miss explanation.
    pub session_cap: usize,
    /// How long a shutdown waits for responses in flight.
    pub drain_timeout: Duration,
    /// The corporate-network settings: HTTP proxy and extra certificate authorities. The default
    /// is none; the CLI fills it from the environment ([`NetworkSettings::from_env`]).
    pub network: NetworkSettings,
    /// Source of request timestamps.
    pub clock: Arc<dyn Clock + Send + Sync>,
    /// `[routing] mode`, stored with every request record. `None` records nothing.
    pub routing_mode: Option<&'static str>,
}

impl ServerConfig {
    /// A configuration with the default session cap, drain timeout and the wall clock.
    pub fn new(
        bind: SocketAddr,
        upstream: impl Into<String>,
        admin_secret: impl Into<String>,
    ) -> Self {
        Self {
            bind,
            allow_non_loopback: false,
            upstream: upstream.into(),
            admin_secret: admin_secret.into(),
            session_cap: DEFAULT_SESSION_CAP,
            drain_timeout: DEFAULT_DRAIN_TIMEOUT,
            network: NetworkSettings::default(),
            clock: Arc::new(SystemClock),
            routing_mode: None,
        }
    }

    /// Builds the configuration from the `[proxy]` section and the admin secret the CLI keeps
    /// in `<data>/proxy/secret`.
    pub fn from_proxy_config(
        config: &ProxyConfig,
        admin_secret: impl Into<String>,
    ) -> Result<Self, ProxyError> {
        let bind = config
            .bind
            .trim()
            .parse()
            .map_err(|_| ProxyError::InvalidBind(config.bind.clone()))?;
        Ok(Self::new(bind, config.upstream.clone(), admin_secret))
    }
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerConfig")
            .field("bind", &self.bind)
            .field("allow_non_loopback", &self.allow_non_loopback)
            .field("upstream", &self.upstream)
            .field("admin_secret", &"<redacted>")
            .field("session_cap", &self.session_cap)
            .field("drain_timeout", &self.drain_timeout)
            .field("network", &self.network)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_proxy_section_maps_to_a_server_config() {
        let config = ServerConfig::from_proxy_config(&ProxyConfig::default(), "s3cret").unwrap();
        assert_eq!(config.bind, "127.0.0.1:1029".parse().unwrap());
        assert_eq!(config.upstream, "https://api.anthropic.com");
        assert!(!config.allow_non_loopback);
        assert_eq!(config.session_cap, DEFAULT_SESSION_CAP);
        assert_eq!(config.drain_timeout, DEFAULT_DRAIN_TIMEOUT);

        let bad = ProxyConfig {
            bind: "localhost:x".to_owned(),
            ..ProxyConfig::default()
        };
        assert!(matches!(
            ServerConfig::from_proxy_config(&bad, ""),
            Err(ProxyError::InvalidBind(value)) if value == "localhost:x"
        ));
    }

    #[test]
    fn debug_never_shows_the_secret() {
        let config = ServerConfig::new("127.0.0.1:1".parse().unwrap(), "https://x", "s3cret");
        let text = format!("{config:?}");
        assert!(!text.contains("s3cret"));
        assert!(text.contains("<redacted>") && text.contains("https://x"));
    }

    #[test]
    fn the_system_clock_is_after_2020() {
        assert!(SystemClock.now().0 > 1_577_836_800);
    }
}
