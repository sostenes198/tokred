//! The listening side: binding (loopback only by default), the accept loop, graceful shutdown
//! and the blocking wrapper the CLI uses.

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokred_core::proxy::{ProxyHook, RecordSink};

use crate::config::ServerConfig;
use crate::connector::UpstreamClient;
use crate::error::ProxyError;
use crate::exchange::{self, Shared, SharedParts};
use crate::host::check_bind;
use crate::upstream::Upstream;

/// Pause after a failed `accept` (e.g. too many open files) so the loop does not spin.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);

/// A stop signal for a running proxy. Cloning shares the signal; it can be triggered from any
/// thread, before or after the server starts.
#[derive(Debug, Clone)]
pub struct Shutdown {
    sender: Arc<watch::Sender<bool>>,
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

impl Shutdown {
    /// A signal that has not been triggered.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sender: Arc::new(watch::Sender::new(false)),
        }
    }

    /// Asks the proxy to stop: it stops accepting connections, lets responses in flight finish
    /// (up to the drain timeout) and returns.
    pub fn trigger(&self) {
        self.sender.send_replace(true);
    }

    /// Whether [`Shutdown::trigger`] was called.
    #[must_use]
    pub fn is_triggered(&self) -> bool {
        *self.sender.borrow()
    }

    /// Resolves once triggered.
    pub(crate) async fn wait(&self) {
        let mut receiver = self.sender.subscribe();
        // The sender lives as long as `self`, so waiting cannot fail.
        let _closed = receiver.wait_for(|stopped| *stopped).await;
    }
}

/// A bound, not yet serving, proxy. Binding is synchronous so the caller learns the address
/// (useful with port 0) before serving.
#[derive(Debug)]
pub struct Proxy {
    listener: std::net::TcpListener,
    local_addr: SocketAddr,
    upstream: Upstream,
    client: UpstreamClient,
    config: ServerConfig,
}

impl Proxy {
    /// Validates the configuration and opens the listening socket.
    pub fn bind(config: ServerConfig) -> Result<Self, ProxyError> {
        check_bind(config.bind, config.allow_non_loopback)?;
        let upstream = Upstream::parse(&config.upstream)?;
        let client = UpstreamClient::build(&upstream, &config.network)?;
        let listener =
            std::net::TcpListener::bind(config.bind).map_err(|source| ProxyError::Bind {
                addr: config.bind,
                source,
            })?;
        listener.set_nonblocking(true)?;
        let local_addr = listener.local_addr()?;
        Ok(Self {
            listener,
            local_addr,
            upstream,
            client,
            config,
        })
    }

    /// The address actually bound.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Serves until `shutdown` is triggered (or `POST /__tokred/shutdown` is called). Must run
    /// inside a multi-thread tokio runtime.
    pub async fn serve(
        self,
        hooks: Vec<Arc<dyn ProxyHook>>,
        sink: Arc<dyn RecordSink>,
        shutdown: Shutdown,
    ) -> Result<(), ProxyError> {
        let listener = TcpListener::from_std(self.listener)?;
        let shared = Arc::new(Shared::new(SharedParts {
            addr: self.local_addr,
            upstream: self.upstream,
            client: self.client,
            hooks,
            sink,
            clock: self.config.clock,
            secret: self.config.admin_secret,
            session_cap: self.config.session_cap,
            shutdown: shutdown.clone(),
        }));
        tracing::info!(addr = %self.local_addr, upstream = %shared.upstream_display(), "proxy listening");
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                () = shutdown.wait() => break,
                accepted = listener.accept() => {
                    if let Some(stream) = accepted_stream(accepted).await {
                        connections.spawn(serve_connection(stream, shared.clone(), shutdown.clone()));
                    }
                }
            }
            while connections.try_join_next().is_some() {}
        }
        drop(listener);
        let drained = tokio::time::timeout(self.config.drain_timeout, async {
            while connections.join_next().await.is_some() {}
        })
        .await;
        if drained.is_err() {
            tracing::warn!("stopping with responses still in flight after the drain timeout");
        }
        tracing::info!("proxy stopped");
        Ok(())
    }

    /// [`Proxy::serve`] on a new multi-thread runtime, blocking the calling thread.
    pub fn serve_blocking(
        self,
        hooks: Vec<Arc<dyn ProxyHook>>,
        sink: Arc<dyn RecordSink>,
        shutdown: Shutdown,
    ) -> Result<(), ProxyError> {
        runtime()?.block_on(self.serve(hooks, sink, shutdown))
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, ProxyError> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("tokred-proxy")
        .build()?)
}

/// The accepted stream, or `None` after logging the error and backing off.
async fn accepted_stream(accepted: io::Result<(TcpStream, SocketAddr)>) -> Option<TcpStream> {
    match accepted {
        Ok((stream, _peer)) => Some(stream),
        Err(error) => {
            tracing::warn!(%error, "could not accept a connection");
            tokio::time::sleep(ACCEPT_BACKOFF).await;
            None
        }
    }
}

/// Serves one client connection (HTTP/1.1 with keep-alive) until it closes or the proxy stops.
async fn serve_connection(stream: TcpStream, shared: Arc<Shared>, shutdown: Shutdown) {
    let _nodelay = stream.set_nodelay(true);
    let service = service_fn(move |request| exchange::handle(shared.clone(), request));
    let connection = http1::Builder::new().serve_connection(TokioIo::new(stream), service);
    tokio::pin!(connection);
    let result = tokio::select! {
        result = connection.as_mut() => result,
        () = shutdown.wait() => {
            connection.as_mut().graceful_shutdown();
            connection.await
        }
    };
    if let Err(error) = result {
        tracing::debug!(%error, "client connection ended with an error");
    }
}

/// Binds and serves until `shutdown` (see [`Proxy`] to learn the bound address first).
pub async fn run(
    config: ServerConfig,
    hooks: Vec<Arc<dyn ProxyHook>>,
    sink: Arc<dyn RecordSink>,
    shutdown: Shutdown,
) -> Result<(), ProxyError> {
    Proxy::bind(config)?.serve(hooks, sink, shutdown).await
}

/// [`run`] on a new multi-thread runtime, blocking the calling thread.
pub fn run_blocking(
    config: ServerConfig,
    hooks: Vec<Arc<dyn ProxyHook>>,
    sink: Arc<dyn RecordSink>,
    shutdown: Shutdown,
) -> Result<(), ProxyError> {
    Proxy::bind(config)?.serve_blocking(hooks, sink, shutdown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_failed_accept_is_skipped_after_a_pause() {
        let started = std::time::Instant::now();
        assert!(
            accepted_stream(Err(io::Error::other("too many files")))
                .await
                .is_none()
        );
        assert!(started.elapsed() >= ACCEPT_BACKOFF);
    }

    #[test]
    fn the_shutdown_signal_is_shared_by_clones() {
        let shutdown = Shutdown::default();
        let clone = shutdown.clone();
        assert!(!clone.is_triggered());
        shutdown.trigger();
        assert!(clone.is_triggered());
    }
}
