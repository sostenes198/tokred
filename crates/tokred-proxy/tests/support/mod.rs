//! Test harness: a scripted fake upstream (a small hyper server on 127.0.0.1), a running proxy
//! with an in-memory record sink, and HTTP helpers. No external network is used.
#![allow(dead_code)]

pub mod net;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::channel::Channel;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{HeaderName, HeaderValue};
use hyper::service::service_fn;
use hyper::{HeaderMap, Method, Request, Response};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokred_core::Timestamp;
use tokred_core::ports::Clock;
use tokred_core::proxy::{
    HookFailure, ProxyHook, ProxyRecord, ProxyRequestRecord, RecordSink, RequestCtx,
    ResponseSummary, rewrite_model, rewrite_ticket,
};
use tokred_proxy::{Proxy, ProxyError, ServerConfig, Shutdown};

/// A request as the fake upstream received it.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub uri: String,
    /// Lower-case names, in arrival order.
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// What the fake upstream answers.
#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub chunks: Vec<Bytes>,
    /// Pause after each chunk.
    pub gap: Duration,
    /// End the body with an error (the connection is cut mid-body).
    pub abort: bool,
    /// Send one body with a `content-length` instead of streaming chunks.
    pub sized: bool,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            chunks: vec![Bytes::from(body.to_owned())],
            gap: Duration::ZERO,
            abort: false,
            sized: true,
        }
    }

    pub fn stream(chunks: Vec<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            chunks: chunks.into_iter().map(Bytes::from).collect(),
            gap: Duration::from_millis(2),
            abort: false,
            sized: false,
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

pub type Script = Arc<dyn Fn(&Seen) -> Reply + Send + Sync>;

/// A fake Anthropic API.
pub struct FakeUpstream {
    pub addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
}

type FakeBody = BoxBody<Bytes, std::io::Error>;

fn reply_to_response(reply: Reply) -> Response<FakeBody> {
    let body: FakeBody = if reply.sized {
        Full::new(reply.chunks.concat().into())
            .map_err(|never: Infallible| match never {})
            .boxed()
    } else {
        let (mut sender, body) = Channel::<Bytes, std::io::Error>::new(1);
        tokio::spawn(async move {
            for chunk in reply.chunks {
                if sender.send_data(chunk).await.is_err() {
                    return;
                }
                tokio::time::sleep(reply.gap).await;
            }
            if reply.abort {
                sender.abort(std::io::Error::other("cut"));
            }
        });
        body.boxed()
    };
    let mut response = Response::new(body);
    *response.status_mut() = hyper::StatusCode::from_u16(reply.status).unwrap();
    for (name, value) in reply.headers {
        response.headers_mut().append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(&value).unwrap(),
        );
    }
    response
}

impl FakeUpstream {
    pub async fn start(script: Script) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let script = script.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |request: Request<Incoming>| {
                        let script = script.clone();
                        let log = log.clone();
                        async move {
                            let (parts, body) = request.into_parts();
                            let body = body.collect().await.unwrap().to_bytes();
                            let seen = Seen {
                                method: parts.method.to_string(),
                                uri: parts.uri.to_string(),
                                headers: parts
                                    .headers
                                    .iter()
                                    .map(|(n, v)| {
                                        (
                                            n.as_str().to_owned(),
                                            String::from_utf8_lossy(v.as_bytes()).into_owned(),
                                        )
                                    })
                                    .collect(),
                                body,
                            };
                            let reply = script(&seen);
                            log.lock().unwrap().push(seen);
                            Ok::<_, Infallible>(reply_to_response(reply))
                        }
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self { addr, seen }
    }

    pub async fn always(reply: Reply) -> Self {
        Self::start(Arc::new(move |_| reply.clone())).await
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

/// Collects records in memory.
#[derive(Debug, Default)]
pub struct MemorySink {
    records: Mutex<Vec<ProxyRecord>>,
}

impl RecordSink for MemorySink {
    fn record(&self, record: ProxyRecord) {
        self.records.lock().unwrap().push(record);
    }
}

impl MemorySink {
    pub fn requests(&self) -> Vec<ProxyRequestRecord> {
        self.records
            .lock()
            .unwrap()
            .iter()
            .map(|record| match record {
                ProxyRecord::Request(request) => (**request).clone(),
                other => panic!("unexpected record {other:?}"),
            })
            .collect()
    }

    /// Waits until at least `count` request records arrived (the record is written when the
    /// response body ends, which can be just after the client read its last byte).
    pub async fn wait_for(&self, count: usize) -> Vec<ProxyRequestRecord> {
        for _ in 0..500 {
            let requests = self.requests();
            if requests.len() >= count {
                return requests;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("expected {count} records, got {}", self.requests().len());
    }
}

/// A clock the test moves by hand.
#[derive(Debug, Default)]
pub struct ManualClock(pub AtomicI64);

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.load(Ordering::SeqCst))
    }
}

impl ManualClock {
    pub fn set(&self, secs: i64) {
        self.0.store(secs, Ordering::SeqCst);
    }
}

/// A proxy serving in the background.
pub struct Running {
    pub addr: SocketAddr,
    pub shutdown: Shutdown,
    pub sink: Arc<MemorySink>,
    pub task: JoinHandle<Result<(), ProxyError>>,
}

pub const SECRET: &str = "test-admin-secret";

pub fn config(upstream: &str) -> ServerConfig {
    ServerConfig::new("127.0.0.1:0".parse().unwrap(), upstream, SECRET)
}

/// Must be called inside the runtime (it spawns the server task).
#[allow(clippy::unused_async)]
pub async fn start_with(config: ServerConfig, hooks: Vec<Arc<dyn ProxyHook>>) -> Running {
    let proxy = Proxy::bind(config).unwrap();
    let addr = proxy.local_addr();
    let shutdown = Shutdown::new();
    let sink = Arc::new(MemorySink::default());
    let task = tokio::spawn(proxy.serve(hooks, sink.clone(), shutdown.clone()));
    Running {
        addr,
        shutdown,
        sink,
        task,
    }
}

pub async fn start(upstream: &str, hooks: Vec<Arc<dyn ProxyHook>>) -> Running {
    start_with(config(upstream), hooks).await
}

impl Running {
    pub async fn stop(self) {
        self.shutdown.trigger();
        self.task.await.unwrap().unwrap();
    }
}

/// A response as the test client received it.
#[derive(Debug)]
pub struct Answer {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl Answer {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

pub fn client() -> Client<HttpConnector, Full<Bytes>> {
    Client::builder(TokioExecutor::new()).build_http()
}

pub fn request(
    addr: SocketAddr,
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
    body: impl Into<Bytes>,
) -> Request<Full<Bytes>> {
    let mut request = Request::new(Full::new(body.into()));
    *request.method_mut() = method;
    *request.uri_mut() = format!("http://{addr}{path}").parse().unwrap();
    for (name, value) in headers {
        request.headers_mut().append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    request
}

pub async fn send(
    addr: SocketAddr,
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
    body: impl Into<Bytes>,
) -> Answer {
    let response = client()
        .request(request(addr, method, path, headers, body))
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    Answer {
        status: parts.status.as_u16(),
        headers: parts.headers,
        body: body.collect().await.unwrap().to_bytes(),
    }
}

pub async fn post_messages(addr: SocketAddr, headers: &[(&str, &str)], body: &[u8]) -> Answer {
    send(
        addr,
        Method::POST,
        "/v1/messages",
        headers,
        Bytes::copy_from_slice(body),
    )
    .await
}

/// Writes raw bytes and reads until the server closes or `limit` bytes arrived.
pub async fn raw(addr: SocketAddr, bytes: &[u8]) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(bytes).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut out)).await;
    String::from_utf8_lossy(&out).into_owned()
}

/// An SSE body for a complete message, as separate event strings.
pub fn sse_events(id: &str, model: &str) -> Vec<String> {
    let events = [
        format!(
            r#"{{"type":"message_start","message":{{"id":"{id}","type":"message","role":"assistant","model":"{model}","content":[],"stop_reason":null,"usage":{{"input_tokens":12,"output_tokens":1,"cache_creation_input_tokens":300,"cache_read_input_tokens":4000,"cache_creation":{{"ephemeral_5m_input_tokens":100,"ephemeral_1h_input_tokens":200}}}}}}}}"#
        ),
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#.to_owned(),
        r#"{"type":"ping"}"#.to_owned(),
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#.to_owned(),
        r#"{"type":"content_block_stop","index":0}"#.to_owned(),
        r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"t1","name":"Read","input":{}}}"#.to_owned(),
        r#"{"type":"content_block_stop","index":1}"#.to_owned(),
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":42,"output_tokens_details":{"thinking_tokens":17}}}"#.to_owned(),
        r#"{"type":"message_stop"}"#.to_owned(),
    ];
    events
        .iter()
        .map(|data| {
            let name = serde_json::from_str::<serde_json::Value>(data).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned();
            format!("event: {name}\ndata: {data}\n\n")
        })
        .collect()
}

/// The whole SSE body.
pub fn sse_body(id: &str, model: &str) -> Vec<u8> {
    sse_events(id, model).concat().into_bytes()
}

/// Splits bytes at the given offsets.
pub fn split_at(bytes: &[u8], cuts: &[usize]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut start = 0;
    for &cut in cuts {
        out.push(bytes[start..cut].to_vec());
        start = cut;
    }
    out.push(bytes[start..].to_vec());
    out
}

/// A typical main-loop request body.
pub fn main_body(model: &str, tools: &[&str]) -> Vec<u8> {
    let tools: Vec<serde_json::Value> = tools
        .iter()
        .map(|name| serde_json::json!({"name": name, "input_schema": {"type": "object"}}))
        .collect();
    serde_json::json!({
        "model": model,
        "max_tokens": 32000,
        "stream": true,
        "thinking": {"type": "adaptive"},
        "system": [{"type": "text", "text": "You are Claude Code.", "cache_control": {"type": "ephemeral"}}],
        "tools": tools,
        "messages": [
            {"role": "user", "content": "find the bug"},
            {"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "Read", "input": {}}]},
            {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "fn main() {}"}]}
        ]
    })
    .to_string()
    .into_bytes()
}

/// What a recording hook saw.
#[derive(Debug, Default)]
pub struct Calls {
    pub before: Mutex<Vec<String>>,
    pub after: Mutex<Vec<ResponseSummary>>,
    pub fallback: Mutex<Vec<(String, u16)>>,
}

/// What the test hook does in `before`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behavior {
    Observe,
    Rewrite,
    Fail,
    Panic,
}

/// A hook for tests: observes, rewrites subagent models to Haiku, fails or panics.
pub struct TestHook {
    pub behavior: Behavior,
    pub calls: Arc<Calls>,
}

pub const REWRITTEN: &str = "claude-haiku-5-5";

impl ProxyHook for TestHook {
    fn name(&self) -> &'static str {
        "test-hook"
    }

    fn before(&self, ctx: &mut RequestCtx<'_>) -> Result<(), HookFailure> {
        self.calls
            .before
            .lock()
            .unwrap()
            .push(ctx.origin.name().to_owned());
        match self.behavior {
            Behavior::Observe => Ok(()),
            Behavior::Rewrite => {
                if let Some(ticket) = rewrite_ticket(ctx.origin) {
                    rewrite_model(&mut ctx.body, &ticket, REWRITTEN);
                    ctx.changed = true;
                    ctx.notes.model_rewritten = Some(REWRITTEN.to_owned());
                    ctx.notes.decision_id = Some("d1".to_owned());
                }
                Ok(())
            }
            Behavior::Fail => Err(HookFailure {
                hook: "test-hook",
                message: "cannot decide".to_owned(),
            }),
            Behavior::Panic => panic!("hook bug"),
        }
    }

    fn after(&self, _ctx: &RequestCtx<'_>, response: &ResponseSummary) {
        self.calls.after.lock().unwrap().push(response.clone());
    }

    fn fallback(&self, ctx: &RequestCtx<'_>, status: u16) {
        let model = ctx.body["model"].as_str().unwrap_or_default().to_owned();
        self.calls.fallback.lock().unwrap().push((model, status));
    }
}

pub fn hook(behavior: Behavior) -> (Arc<dyn ProxyHook>, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    let hook = Arc::new(TestHook {
        behavior,
        calls: calls.clone(),
    });
    (hook, calls)
}

/// A port nothing listens on.
pub async fn closed_port() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap()
}
