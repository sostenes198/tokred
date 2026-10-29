//! The local proxy (spec section 6): listens on loopback, forwards every request to the
//! configured upstream with streaming, runs the hook pipeline of `tokred_core::proxy` on
//! `POST /v1/messages`, observes the responses to measure usage and latency, and hands one
//! [`ProxyRecord`](tokred_core::proxy::ProxyRecord) per message request to a
//! [`RecordSink`](tokred_core::proxy::RecordSink).
//!
//! ```no_run
//! use std::sync::Arc;
//! use tokred_proxy::{Proxy, ServerConfig, Shutdown, spawn_writer};
//!
//! # fn main() -> Result<(), tokred_proxy::ProxyError> {
//! let config = ServerConfig::new("127.0.0.1:1029".parse().unwrap(), "https://api.anthropic.com", "secret");
//! let (sink, writer) = spawn_writer(Box::new(|_record| Ok(())));
//! let proxy = Proxy::bind(config)?;
//! let shutdown = Shutdown::new();
//! proxy.serve_blocking(Vec::new(), Arc::new(sink), shutdown)?;
//! # let _ = writer.join();
//! # Ok(())
//! # }
//! ```

mod admin;
mod config;
mod connector;
mod error;
mod exchange;
mod headers;
mod host;
mod miss;
mod network;
mod pipeline;
mod record;
mod server;
mod sse;
mod tap;
mod upstream;
mod writer;

pub use config::{DEFAULT_DRAIN_TIMEOUT, DEFAULT_SESSION_CAP, ServerConfig, SystemClock};
pub use error::ProxyError;
pub use miss::{MainTurn, MissCause, explain_cache_miss};
pub use network::{CaFileSummary, NetworkSettings, NetworkSummary, ProxySummary};
pub use server::{Proxy, Shutdown, run, run_blocking};
pub use writer::{
    ApplyRecord, ChannelSink, DEFAULT_WRITER_CAPACITY, WriterReport, spawn_writer,
    spawn_writer_with_capacity,
};

/// Version reported by `/__tokred/health` and `/__tokred/status`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Path of the health endpoint (no secret needed).
pub const HEALTH_PATH: &str = "/__tokred/health";
/// Path of the status endpoint (needs the secret).
pub const STATUS_PATH: &str = "/__tokred/status";
/// Path of the shutdown endpoint (`POST`, needs the secret).
pub const SHUTDOWN_PATH: &str = "/__tokred/shutdown";
/// Header carrying the admin secret.
pub const SECRET_HEADER: &str = admin::SECRET_HEADER;
