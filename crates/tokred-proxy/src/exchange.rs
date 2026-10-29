//! One request through the proxy: Host check, admin endpoints, forwarding with streaming, the
//! `/v1/messages` pipeline with its fallback, and the record written once the response ends.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Instant;

use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Frame, Incoming, SizeHint};
use hyper::header::{CONTENT_TYPE, HOST, HeaderValue};
use hyper::http::request::Parts;
use hyper::http::response;
use hyper::{HeaderMap, Method, Request, Response, StatusCode};
use serde_json::{Value, json};
use tokred_core::Timestamp;
use tokred_core::ports::Clock;
use tokred_core::proxy::{
    Origin, ProxyHook, ProxyRecord, ProxyRequestRecord, RecordSink, ResponseSummary,
};

use crate::admin::{self, Admin};
use crate::connector::UpstreamClient;
use crate::headers;
use crate::host::host_allowed;
use crate::miss::{MainTurn, SessionTracker};
use crate::pipeline::{self, Prepared};
use crate::record::{
    canonical_hash, message_key, micros, millis, new_request_id, thinking_signature,
};
use crate::server::Shutdown;
use crate::tap::{Observed, ResponseTap};
use crate::upstream::Upstream;

/// Body of every response the proxy returns.
pub(crate) type ProxyBody = UnsyncBoxBody<Bytes, hyper::Error>;

/// Statuses that trigger the resend of the original body after a rewrite (spec 5.5).
const REJECTIONS: [u16; 3] = [400, 404, 422];

/// Counters shown by `/__tokred/status`.
#[derive(Debug, Default)]
struct Counters {
    requests: AtomicU64,
    messages: AtomicU64,
    hook_failures: AtomicU64,
    fallbacks: AtomicU64,
    upstream_errors: AtomicU64,
}

fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

/// State shared by every connection.
pub(crate) struct Shared {
    addr: SocketAddr,
    upstream: Upstream,
    client: UpstreamClient,
    hooks: Vec<Arc<dyn ProxyHook>>,
    sink: Arc<dyn RecordSink>,
    clock: Arc<dyn Clock + Send + Sync>,
    secret: String,
    tracker: Mutex<SessionTracker>,
    counters: Counters,
    started: Instant,
    shutdown: Shutdown,
}

/// What [`Shared::new`] needs.
pub(crate) struct SharedParts {
    pub addr: SocketAddr,
    pub upstream: Upstream,
    pub client: UpstreamClient,
    pub hooks: Vec<Arc<dyn ProxyHook>>,
    pub sink: Arc<dyn RecordSink>,
    pub clock: Arc<dyn Clock + Send + Sync>,
    pub secret: String,
    pub session_cap: usize,
    pub shutdown: Shutdown,
}

/// The error and its causes, for logs (never contains headers or bodies).
fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

impl Shared {
    pub(crate) fn new(parts: SharedParts) -> Self {
        Self {
            addr: parts.addr,
            upstream: parts.upstream,
            client: parts.client,
            hooks: parts.hooks,
            sink: parts.sink,
            clock: parts.clock,
            secret: parts.secret,
            tracker: Mutex::new(SessionTracker::new(parts.session_cap)),
            counters: Counters::default(),
            started: Instant::now(),
            shutdown: parts.shutdown,
        }
    }

    /// `scheme://authority/base` of the upstream.
    pub(crate) fn upstream_display(&self) -> String {
        self.upstream.display()
    }

    /// Sends a request upstream with the client's headers (minus hop-by-hop ones).
    async fn send(&self, parts: &Parts, body: Bytes) -> Result<Response<Incoming>, String> {
        let mut request = Request::new(Full::new(body));
        *request.method_mut() = parts.method.clone();
        *request.uri_mut() = self.upstream.target(&parts.uri)?;
        *request.headers_mut() = headers::upstream_request(&parts.headers);
        self.client
            .request(request)
            .await
            .map_err(|error| describe(&error))
    }

    fn status(&self) -> Value {
        let count = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        let sessions = self
            .tracker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let hooks: Vec<&str> = self.hooks.iter().map(|hook| hook.name()).collect();
        json!({
            "ok": true,
            "version": crate::VERSION,
            "listen": self.addr.to_string(),
            "upstream": self.upstream.display(),
            "uptime_s": self.started.elapsed().as_secs(),
            "requests": count(&self.counters.requests),
            "messages": count(&self.counters.messages),
            "hook_failures": count(&self.counters.hook_failures),
            "fallbacks": count(&self.counters.fallbacks),
            "upstream_errors": count(&self.counters.upstream_errors),
            "sessions": sessions,
            "hooks": hooks,
        })
    }
}

fn full(bytes: impl Into<Bytes>) -> ProxyBody {
    Full::new(bytes.into())
        .map_err(|never: Infallible| match never {})
        .boxed_unsync()
}

fn json_response(status: StatusCode, value: &Value) -> Response<ProxyBody> {
    let mut response = Response::new(full(value.to_string()));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

/// An error answered by the proxy itself, in the API's error shape.
fn local_error(status: StatusCode, kind: &str, message: &str) -> Response<ProxyBody> {
    json_response(
        status,
        &json!({"type": "error", "error": {"type": kind, "message": message}}),
    )
}

fn bad_gateway(shared: &Shared, error: &str) -> Response<ProxyBody> {
    bump(&shared.counters.upstream_errors);
    tracing::warn!(%error, "the upstream request failed");
    local_error(
        StatusCode::BAD_GATEWAY,
        "api_error",
        "tokred proxy: the upstream could not be reached",
    )
}

fn admin_response(shared: &Shared, action: Admin, headers: &HeaderMap) -> Response<ProxyBody> {
    let allowed = action == Admin::Health
        || (matches!(action, Admin::Status | Admin::Shutdown)
            && admin::authorized(headers, &shared.secret));
    if !allowed {
        let (status, message) = admin::refusal(action);
        return json_response(status, &json!({"ok": false, "error": message}));
    }
    match action {
        Admin::Shutdown => {
            tracing::info!("shutdown requested through the admin endpoint");
            shared.shutdown.trigger();
            json_response(StatusCode::OK, &json!({"ok": true, "stopping": true}))
        }
        Admin::Status => json_response(StatusCode::OK, &shared.status()),
        _ => json_response(
            StatusCode::OK,
            &json!({"ok": true, "version": crate::VERSION}),
        ),
    }
}

/// The client's response: the upstream's status and headers (minus hop-by-hop ones) and body.
fn to_client(mut head: response::Parts, body: ProxyBody) -> Response<ProxyBody> {
    head.headers = headers::client_response(&head.headers);
    Response::from_parts(head, body)
}

/// Entry point of every request (hyper service).
pub(crate) async fn handle(
    shared: Arc<Shared>,
    request: Request<Incoming>,
) -> Result<Response<ProxyBody>, Infallible> {
    Ok(respond(&shared, request).await)
}

async fn respond(shared: &Arc<Shared>, request: Request<Incoming>) -> Response<ProxyBody> {
    let received = Instant::now();
    bump(&shared.counters.requests);
    if !host_allowed(headers::text(request.headers(), HOST.as_str()), shared.addr) {
        return local_error(
            StatusCode::MISDIRECTED_REQUEST,
            "invalid_request_error",
            "tokred proxy: requests must address the proxy by a loopback host",
        );
    }
    if let Some(action) = admin::route(request.method(), request.uri().path()) {
        return admin_response(shared, action, request.headers());
    }
    let (parts, body) = request.into_parts();
    let Ok(collected) = body.collect().await else {
        return local_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "tokred proxy: the request body could not be read",
        );
    };
    let original = collected.to_bytes();
    if parts.method == Method::POST && parts.uri.path() == "/v1/messages" {
        messages(shared, &parts, original, received).await
    } else {
        match shared.send(&parts, original).await {
            Ok(response) => {
                let (head, body) = response.into_parts();
                to_client(head, body.boxed_unsync())
            }
            Err(error) => bad_gateway(shared, &error),
        }
    }
}

/// `POST /v1/messages`: hooks, forwarding, fallback and the response tap.
async fn messages(
    shared: &Arc<Shared>,
    parts: &Parts,
    original: Bytes,
    received: Instant,
) -> Response<ProxyBody> {
    bump(&shared.counters.messages);
    let ts = shared.clock.now();
    let mut prepared = pipeline::prepare(&parts.headers, &original, &shared.hooks, ts);
    if prepared.failed {
        bump(&shared.counters.hook_failures);
    }
    let overhead_us = micros(received.elapsed());
    let mut bytes_out = prepared.send.len();
    let response = match shared.send(parts, prepared.send.clone()).await {
        Ok(rejected) if prepared.changed && REJECTIONS.contains(&rejected.status().as_u16()) => {
            let status = rejected.status().as_u16();
            drop(rejected);
            bump(&shared.counters.fallbacks);
            tracing::warn!(
                status,
                "the upstream rejected a rewritten request; resending the original"
            );
            pipeline::run_fallback(&shared.hooks, &mut prepared, ts, status);
            bytes_out = original.len();
            shared.send(parts, original.clone()).await
        }
        other => other,
    };
    let exchange = Exchange {
        id: new_request_id(),
        ts,
        prepared,
        received,
        overhead_us,
        bytes_in: original.len() as u64,
        bytes_out: bytes_out as u64,
    };
    let ttfb_ms = millis(received.elapsed());
    match response {
        Ok(response) => {
            let (head, body) = response.into_parts();
            let finish = Finish {
                shared: shared.clone(),
                exchange,
                status: head.status.as_u16(),
                request_id: headers::text(&head.headers, "request-id").map(str::to_owned),
                ttfb_ms,
            };
            let tap = ResponseTap::new(&head.headers);
            let body = TapBody {
                inner: body,
                tap: Some(tap),
                finish: Some(finish),
            };
            to_client(head, body.boxed_unsync())
        }
        Err(error) => {
            let finish = Finish {
                shared: shared.clone(),
                exchange,
                status: StatusCode::BAD_GATEWAY.as_u16(),
                request_id: None,
                ttfb_ms,
            };
            finish.complete(Observed::default());
            bad_gateway(shared, &error)
        }
    }
}

/// The request side of a `/v1/messages` exchange, kept until the response ends.
struct Exchange {
    id: String,
    ts: Timestamp,
    prepared: Prepared,
    received: Instant,
    overhead_us: u64,
    bytes_in: u64,
    bytes_out: u64,
}

/// Everything needed to close an exchange: the hooks' `after`, the miss explanation and the
/// record.
struct Finish {
    shared: Arc<Shared>,
    exchange: Exchange,
    status: u16,
    request_id: Option<String>,
    ttfb_ms: u64,
}

impl Finish {
    fn complete(mut self, observed: Observed) {
        let summary = ResponseSummary {
            status: self.status,
            model: observed.model,
            message_id: observed.message_id,
            request_id: self.request_id.take(),
            usage: observed.usage,
            thinking_tokens: observed.thinking_tokens,
            stop_reason: observed.stop_reason,
            tool_uses: observed.tool_uses,
            ttfb_ms: self.ttfb_ms,
            total_ms: millis(self.exchange.received.elapsed()),
            interrupted: observed.interrupted,
        };
        let record = self.record(&summary);
        pipeline::run_after(
            &self.shared.hooks,
            &mut self.exchange.prepared,
            self.exchange.ts,
            &summary,
        );
        self.shared
            .sink
            .record(ProxyRecord::Request(Box::new(record)));
    }

    fn record(&self, summary: &ResponseSummary) -> ProxyRequestRecord {
        let exchange = &self.exchange;
        let prepared = &exchange.prepared;
        let body = prepared.body.as_ref();
        let hash = |key: &str| body.map_or_else(String::new, |body| canonical_hash(body.get(key)));
        let tools_hash = hash("tools");
        let system_hash = hash("system");
        let miss_cause = self.miss_cause(summary, &tools_hash, &system_hash);
        ProxyRequestRecord {
            id: exchange.id.clone(),
            ts: exchange.ts,
            session_id: prepared.meta.session_id.clone().unwrap_or_default(),
            agent_id: prepared.meta.agent_id.clone(),
            agent_type: prepared.meta.agent_type.clone(),
            prompt_id: prepared.meta.prompt_id.clone(),
            origin: prepared.origin.name().to_owned(),
            model_requested: prepared.shape.model.clone().unwrap_or_default(),
            model_effective: summary.model.clone().unwrap_or_default(),
            usage: summary.usage,
            thinking_tokens: summary.thinking_tokens,
            message_key: message_key(summary.message_id.as_deref(), summary.request_id.as_deref()),
            bytes_in: exchange.bytes_in,
            bytes_out: exchange.bytes_out,
            upstream_ttfb_ms: summary.ttfb_ms,
            total_ms: summary.total_ms,
            proxy_overhead_us: exchange.overhead_us,
            status: summary.status,
            notes: prepared.notes.clone(),
            miss_cause,
            tools_hash,
            system_hash,
        }
    }

    /// Main-loop requests with a session and a successful message feed the session tracker.
    fn miss_cause(
        &self,
        summary: &ResponseSummary,
        tools_hash: &str,
        system_hash: &str,
    ) -> Option<String> {
        let prepared = &self.exchange.prepared;
        let (Origin::Main, Some(session), Some(body), Some(_), true) = (
            &prepared.origin,
            prepared.meta.session_id.as_deref(),
            prepared.body.as_ref(),
            summary.message_id.as_ref(),
            (200..300).contains(&summary.status),
        ) else {
            return None;
        };
        let turn = MainTurn {
            at: self.exchange.ts,
            model: prepared.shape.model.clone().unwrap_or_default(),
            tools_hash: tools_hash.to_owned(),
            system_hash: system_hash.to_owned(),
            thinking_signature: thinking_signature(body),
            message_count: prepared.shape.message_count,
            compacted: prepared.meta.compaction.is_some()
                || prepared.meta.context_compacted.is_some(),
            uses_1h_cache: summary.usage.cache_write_1h > 0,
            usage: summary.usage,
        };
        let mut tracker = self
            .shared
            .tracker
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        tracker
            .observe(session, turn)
            .map(|cause| cause.as_str().to_owned())
    }
}

/// The upstream body, forwarded frame by frame while the tap reads it. The exchange is closed
/// exactly once: at the end of the body, on a read error, or when the client goes away.
struct TapBody {
    inner: Incoming,
    tap: Option<ResponseTap>,
    finish: Option<Finish>,
}

impl TapBody {
    fn end(&mut self, complete: bool) {
        if let (Some(tap), Some(finish)) = (self.tap.take(), self.finish.take()) {
            finish.complete(tap.finish(complete));
        }
    }
}

impl Body for TapBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, hyper::Error>>> {
        let polled = Pin::new(&mut self.inner).poll_frame(cx);
        match &polled {
            Poll::Ready(Some(Ok(frame))) => {
                if let (Some(tap), Some(data)) = (self.tap.as_mut(), frame.data_ref()) {
                    tap.feed(data);
                }
            }
            Poll::Ready(Some(Err(_))) => self.end(false),
            Poll::Ready(None) => self.end(true),
            Poll::Pending => {}
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

impl Drop for TapBody {
    fn drop(&mut self) {
        let complete = self.inner.is_end_stream();
        self.end(complete);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Chain(std::io::Error);

    impl std::fmt::Display for Chain {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("client error (Connect)")
        }
    }

    impl std::error::Error for Chain {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn errors_are_described_with_their_causes() {
        assert_eq!(describe(&std::io::Error::other("refused")), "refused");
        assert_eq!(
            describe(&Chain(std::io::Error::other("connection refused"))),
            "client error (Connect): connection refused"
        );
    }
}
