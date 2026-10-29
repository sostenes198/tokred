//! The dashboard's HTTP routes, answered by the proxy and never forwarded upstream:
//! `/metrics`, `/dashboard` (page, `app.css`, `app.js`), the JSON API under `/dashboard/api`
//! and the event stream at `/dashboard/events`.
//!
//! Every request goes through, in this order: the Host check (in `exchange`), a refusal of
//! requests that come from another site (403), the method (405), and the browser session (401,
//! except `GET /dashboard?token=...`, which trades a one-time token for a cookie). No CORS
//! headers are ever sent.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::channel::{Channel, Sender};
use http_body_util::{BodyExt, Full};
use hyper::header::{
    ALLOW, CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HeaderValue, LOCATION,
    REFERRER_POLICY, SET_COOKIE, X_CONTENT_TYPE_OPTIONS,
};
use hyper::{HeaderMap, Method, Response, StatusCode};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::broadcast::{Receiver, error::RecvError};
use tokred_core::Timestamp;

use crate::auth::{cookie_value, set_cookie};
use crate::bus::{Event, PING_FRAME};
use crate::dashboard::Dashboard;
use crate::exchange::ProxyBody;
use crate::merge::{merge_detail, merge_list};
use crate::server::Shutdown;

/// The browser app, embedded in the binary (no CDN, no external request).
const PAGE: &str = include_str!("../assets/dashboard.html");
const CSS: &str = include_str!("../assets/app.css");
const JS: &str = include_str!("../assets/app.js");

/// What the page may load and do: its own files and its own event stream, nothing else.
const DASHBOARD_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'";

/// How often an idle event stream sends a comment so proxies and browsers keep it open.
const PING_EVERY: Duration = Duration::from_secs(15);

/// A short page for a browser without a session.
const SIGN_IN: &str = "<!doctype html><meta charset=\"utf-8\"><title>tokred</title><p>The tokred dashboard needs a sign-in: run <code>tokred dashboard</code> in a terminal to open it.</p>";

/// The dashboard resource a path names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Route {
    /// `/metrics`: the HTML report.
    Metrics,
    /// `/dashboard`: the single-page app.
    Page,
    /// `/dashboard/app.css`.
    Css,
    /// `/dashboard/app.js`.
    Js,
    /// `/dashboard/api/sessions`.
    Sessions,
    /// `/dashboard/api/sessions/{id}`.
    Session(String),
    /// `/dashboard/events`: server-sent events.
    Events,
    /// `/favicon.ico` and `/dashboard/favicon.ico`: nothing to show (204).
    Favicon,
    /// Another path under `/dashboard/`.
    Missing,
}

/// The dashboard route of a path, or `None` when the path is not the dashboard's (it is then
/// forwarded like any other request).
pub(crate) fn route(path: &str) -> Option<Route> {
    match path {
        "/metrics" => Some(Route::Metrics),
        "/dashboard" => Some(Route::Page),
        "/favicon.ico" | "/dashboard/favicon.ico" => Some(Route::Favicon),
        _ => {
            let rest = path.strip_prefix("/dashboard/")?;
            Some(match rest {
                "app.css" => Route::Css,
                "app.js" => Route::Js,
                "api/sessions" => Route::Sessions,
                "events" => Route::Events,
                _ => match rest.strip_prefix("api/sessions/") {
                    Some(id) if !id.is_empty() && !id.contains('/') => {
                        Route::Session(id.to_owned())
                    }
                    _ => Route::Missing,
                },
            })
        }
    }
}

/// `Sec-Fetch-Site` says another origin, or another site, asked for this.
fn from_another_site(headers: &HeaderMap) -> bool {
    headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|site| {
            site.eq_ignore_ascii_case("cross-site") || site.eq_ignore_ascii_case("same-site")
        })
}

/// The value of a query parameter (taken verbatim: tokens and session ids need no decoding).
fn query_param<'a>(query: Option<&'a str>, name: &str) -> Option<&'a str> {
    query?
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

/// A response with the headers every dashboard answer carries.
fn reply(
    status: StatusCode,
    content_type: &'static str,
    body: impl Into<Bytes>,
    csp: &str,
) -> Response<ProxyBody> {
    let mut response = Response::new(
        Full::new(body.into())
            .map_err(|never: Infallible| match never {})
            .boxed_unsync(),
    );
    *response.status_mut() = status;
    secure(&mut response, content_type, csp);
    response
}

fn secure(response: &mut Response<ProxyBody>, content_type: &'static str, csp: &str) {
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_str(csp).unwrap_or_else(|_| HeaderValue::from_static(DASHBOARD_CSP)),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

fn text(status: StatusCode, message: &'static str) -> Response<ProxyBody> {
    reply(status, "text/plain; charset=utf-8", message, DASHBOARD_CSP)
}

fn json<T: Serialize>(status: StatusCode, value: &T) -> Response<ProxyBody> {
    let body = serde_json::to_vec(value).unwrap_or_default();
    reply(status, "application/json", body, DASHBOARD_CSP)
}

/// The request needs a browser session it does not have.
fn sign_in() -> Response<ProxyBody> {
    reply(
        StatusCode::UNAUTHORIZED,
        "text/html; charset=utf-8",
        SIGN_IN,
        DASHBOARD_CSP,
    )
}

fn method_not_allowed(allow: &'static str) -> Response<ProxyBody> {
    let mut response = text(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
    response
        .headers_mut()
        .insert(ALLOW, HeaderValue::from_static(allow));
    response
}

/// Runs `work` on a blocking thread (it reads the database); a panic reads as an error.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|error| Err(error.to_string()))
}

/// The store could not answer: logged in full, reported briefly.
fn unavailable(error: &str) -> Response<ProxyBody> {
    tracing::warn!(%error, "the dashboard could not read the store");
    json(
        StatusCode::INTERNAL_SERVER_ERROR,
        &serde_json::json!({"error": "the data could not be read"}),
    )
}

/// What a request to the dashboard says, apart from its body.
pub(crate) struct Asked<'a> {
    pub method: &'a Method,
    pub headers: &'a HeaderMap,
    pub query: Option<&'a str>,
    pub now: Timestamp,
}

/// Answers a dashboard request.
pub(crate) async fn handle(
    dashboard: &Arc<Dashboard>,
    route: Route,
    asked: &Asked<'_>,
) -> Response<ProxyBody> {
    if from_another_site(asked.headers) {
        return text(StatusCode::FORBIDDEN, "cross-site requests are refused");
    }
    let favicon = route == Route::Favicon;
    let allowed = *asked.method == Method::GET || (favicon && *asked.method == Method::HEAD);
    if !allowed {
        return method_not_allowed(if favicon { "GET, HEAD" } else { "GET" });
    }
    let now = Instant::now();
    if route == Route::Page
        && let Some(cookie) =
            query_param(asked.query, "token").and_then(|token| dashboard.auth.redeem(token, now))
    {
        return signed_in(&cookie);
    }
    let signed =
        cookie_value(asked.headers).is_some_and(|cookie| dashboard.auth.session_valid(cookie, now));
    if !signed {
        return sign_in();
    }
    match route {
        Route::Page => reply(
            StatusCode::OK,
            "text/html; charset=utf-8",
            PAGE,
            DASHBOARD_CSP,
        ),
        Route::Css => reply(
            StatusCode::OK,
            "text/css; charset=utf-8",
            CSS,
            DASHBOARD_CSP,
        ),
        Route::Js => reply(
            StatusCode::OK,
            "text/javascript; charset=utf-8",
            JS,
            DASHBOARD_CSP,
        ),
        Route::Favicon => {
            let mut empty = text(StatusCode::NO_CONTENT, "");
            empty.headers_mut().remove(CONTENT_TYPE);
            empty
        }
        Route::Metrics => metrics(dashboard).await,
        Route::Sessions => sessions(dashboard, asked.now).await,
        Route::Session(id) => session_detail(dashboard, id, asked.now).await,
        Route::Events => events(dashboard, query_param(asked.query, "session")),
        Route::Missing => text(StatusCode::NOT_FOUND, "not found"),
    }
}

/// The cookie is set and the browser goes to the page without the token in its URL.
fn signed_in(cookie: &str) -> Response<ProxyBody> {
    let mut response = text(StatusCode::SEE_OTHER, "");
    let headers = response.headers_mut();
    headers.insert(LOCATION, HeaderValue::from_static("/dashboard"));
    if let Ok(value) = HeaderValue::from_str(&set_cookie(cookie)) {
        headers.insert(SET_COOKIE, value);
    }
    response
}

async fn metrics(dashboard: &Arc<Dashboard>) -> Response<ProxyBody> {
    let source = dashboard.source.clone();
    match blocking(move || source.metrics_html()).await {
        Ok(html) => {
            let csp = metrics_csp(&html);
            reply(StatusCode::OK, "text/html; charset=utf-8", html, &csp)
        }
        Err(error) => unavailable(&error),
    }
}

async fn sessions(dashboard: &Arc<Dashboard>, now: Timestamp) -> Response<ProxyBody> {
    let source = dashboard.source.clone();
    match blocking(move || source.sessions()).await {
        Ok(stored) => json(
            StatusCode::OK,
            &merge_list(stored, &dashboard.live.sessions(), now),
        ),
        Err(error) => unavailable(&error),
    }
}

async fn session_detail(
    dashboard: &Arc<Dashboard>,
    id: String,
    now: Timestamp,
) -> Response<ProxyBody> {
    let source = dashboard.source.clone();
    let wanted = id.clone();
    match blocking(move || source.session(&wanted)).await {
        Ok(stored) => match merge_detail(stored, dashboard.live.session(&id).as_ref(), now) {
            Some(detail) => json(StatusCode::OK, &detail),
            None => json(
                StatusCode::NOT_FOUND,
                &serde_json::json!({"error": "unknown session"}),
            ),
        },
        Err(error) => unavailable(&error),
    }
}

/// A `text/event-stream` of the bus, for one session when `?session=` names it. The stream
/// ends when the client leaves or the proxy shuts down.
fn events(dashboard: &Arc<Dashboard>, only: Option<&str>) -> Response<ProxyBody> {
    let (sender, body) = Channel::<Bytes, Infallible>::new(16);
    let receiver = dashboard.bus.subscribe();
    tokio::spawn(pump(
        receiver,
        sender,
        only.map(str::to_owned),
        dashboard.shutdown.clone(),
        PING_EVERY,
    ));
    let mut response = Response::new(
        body.map_err(|never: Infallible| match never {})
            .boxed_unsync(),
    );
    secure(&mut response, "text/event-stream", DASHBOARD_CSP);
    response
}

/// Copies bus events to one client until it leaves (a send fails), the bus closes or the proxy
/// stops. Sends a comment every `ping` while idle. An event lost to a slow client is skipped.
pub(crate) async fn pump(
    mut events: Receiver<Arc<Event>>,
    mut client: Sender<Bytes, Infallible>,
    only: Option<String>,
    shutdown: Shutdown,
    ping: Duration,
) {
    let mut idle = tokio::time::interval_at(tokio::time::Instant::now() + ping, ping);
    let wanted = |event: &Event| only.as_ref().is_none_or(|id| *id == event.session_id);
    let first = Bytes::from_static(b"retry: 3000\n\n");
    let mut next = Some(first);
    loop {
        let frame = if let Some(first) = next.take() {
            first
        } else {
            tokio::select! {
                () = shutdown.wait() => return,
                _ = idle.tick() => Bytes::from_static(PING_FRAME),
                received = events.recv() => match received {
                    Ok(event) if wanted(&event) => event.frame.clone(),
                    Ok(_) | Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                },
            }
        };
        tokio::select! {
            () = shutdown.wait() => return,
            sent = client.send_data(frame) => if sent.is_err() { return },
        }
    }
}

/// The policy of the HTML report: its inline styles, and only the inline scripts it has (by
/// hash); nothing else may run or load.
pub(crate) fn metrics_csp(html: &str) -> String {
    let mut scripts = String::new();
    for source in inline_scripts(html) {
        scripts.push_str(" 'sha256-");
        scripts.push_str(&base64(&Sha256::digest(source.as_bytes())));
        scripts.push('\'');
    }
    format!(
        "default-src 'none'; style-src 'unsafe-inline'; script-src{}; base-uri 'none'; frame-ancestors 'none'",
        if scripts.is_empty() {
            " 'none'"
        } else {
            &scripts
        }
    )
}

/// The text of every `<script>` that runs (data blocks such as `type="application/json"` do not).
fn inline_scripts(html: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find("<script") {
        let after = &rest[open..];
        let Some(tag_end) = after.find('>') else {
            break;
        };
        let (tag, body) = after.split_at(tag_end + 1);
        let Some(close) = body.find("</script>") else {
            break;
        };
        if !tag.contains("type=\"application/json\"") && !tag.contains("src=") {
            found.push(&body[..close]);
        }
        rest = &body[close + "</script>".len()..];
    }
    found
}

/// Standard base64 with padding (what a CSP hash source uses).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let [a, b, c] = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let group = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);
        for (position, shift) in [18_u32, 12, 6, 0].into_iter().enumerate() {
            if position <= chunk.len() {
                out.push(char::from(ALPHABET[((group >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::bus::EventBus;

    type Body = Channel<Bytes, Infallible>;

    fn start_pump(
        events: Receiver<Arc<Event>>,
        only: Option<&str>,
        shutdown: &Shutdown,
        buffer: usize,
    ) -> (Body, tokio::task::JoinHandle<()>) {
        let (sender, body) = Channel::<Bytes, Infallible>::new(buffer);
        let task = tokio::spawn(pump(
            events,
            sender,
            only.map(str::to_owned),
            shutdown.clone(),
            PING_EVERY,
        ));
        (body, task)
    }

    async fn next_text(body: &mut Body) -> Option<String> {
        let frame = body.frame().await?.ok()?;
        Some(String::from_utf8_lossy(frame.data_ref()?).into_owned())
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_stream_pings_and_a_session_filter_drops_other_events() {
        let bus = EventBus::new();
        let shutdown = Shutdown::new();
        let (mut body, task) = start_pump(bus.subscribe(), Some("s1"), &shutdown, 16);
        assert_eq!(next_text(&mut body).await.unwrap(), "retry: 3000\n\n");
        assert_eq!(
            next_text(&mut body).await.unwrap(),
            ": ping\n\n",
            "after 15 idle seconds"
        );
        bus.publish("turn_start", "s2", &serde_json::json!({"n": 1}));
        bus.publish("turn_start", "s1", &serde_json::json!({"n": 2}));
        let frame = next_text(&mut body).await.unwrap();
        assert!(
            frame.contains("event: turn_start") && frame.contains("\"n\":2"),
            "{frame}"
        );
        shutdown.trigger();
        task.await.unwrap();
        assert!(next_text(&mut body).await.is_none(), "the stream ended");
    }

    #[tokio::test(start_paused = true)]
    async fn a_client_that_fell_behind_skips_the_events_it_lost() {
        let bus = EventBus::new();
        let shutdown = Shutdown::new();
        let events = bus.subscribe();
        for n in 0..600 {
            bus.publish("turn_delta", "s", &serde_json::json!({"n": n}));
        }
        let (mut body, task) = start_pump(events, None, &shutdown, 1024);
        assert_eq!(next_text(&mut body).await.unwrap(), "retry: 3000\n\n");
        let first = next_text(&mut body).await.unwrap();
        assert!(
            !first.starts_with("id: 1\n"),
            "the oldest events were lost: {first}"
        );
        shutdown.trigger();
        task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn the_stream_ends_when_the_bus_closes_or_the_client_leaves() {
        let bus = EventBus::new();
        let shutdown = Shutdown::new();
        let (_body, task) = start_pump(bus.subscribe(), None, &shutdown, 16);
        drop(bus);
        task.await.unwrap();

        let bus = EventBus::new();
        let (body, task) = start_pump(bus.subscribe(), None, &shutdown, 16);
        drop(body);
        bus.publish("session", "s", &serde_json::json!({}));
        task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_ends_a_stream_stuck_on_a_client_that_does_not_read() {
        let bus = EventBus::new();
        let shutdown = Shutdown::new();
        let (_body, task) = start_pump(bus.subscribe(), None, &shutdown, 1);
        for n in 0..4 {
            bus.publish("turn_delta", "s", &serde_json::json!({"n": n}));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(
            !task.is_finished(),
            "blocked sending to a client that does not read"
        );
        shutdown.trigger();
        task.await.unwrap();
    }

    #[test]
    fn paths_map_to_routes_and_everything_else_is_not_ours() {
        let table = [
            ("/metrics", Some(Route::Metrics)),
            ("/dashboard", Some(Route::Page)),
            ("/dashboard/app.css", Some(Route::Css)),
            ("/dashboard/app.js", Some(Route::Js)),
            ("/dashboard/api/sessions", Some(Route::Sessions)),
            (
                "/dashboard/api/sessions/abc-123",
                Some(Route::Session("abc-123".into())),
            ),
            ("/dashboard/events", Some(Route::Events)),
            ("/favicon.ico", Some(Route::Favicon)),
            ("/dashboard/favicon.ico", Some(Route::Favicon)),
            ("/dashboard/", Some(Route::Missing)),
            ("/dashboard/nope", Some(Route::Missing)),
            ("/dashboard/api/sessions/", Some(Route::Missing)),
            ("/dashboard/api/sessions/a/b", Some(Route::Missing)),
            ("/v1/messages", None),
            ("/metrics/", None),
            ("/dashboards", None),
            ("/", None),
        ];
        for (path, expected) in table {
            assert_eq!(route(path), expected, "{path}");
        }
    }

    #[test]
    fn requests_from_other_sites_are_recognized() {
        let with = |value: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert("sec-fetch-site", HeaderValue::from_static(value));
            headers
        };
        assert!(from_another_site(&with("cross-site")));
        assert!(from_another_site(&with("Same-Site")));
        assert!(!from_another_site(&with("same-origin")));
        assert!(!from_another_site(&with("none")));
        assert!(!from_another_site(&HeaderMap::new()));
    }

    #[test]
    fn query_parameters_are_found_by_name() {
        assert_eq!(query_param(Some("token=abc&x=1"), "token"), Some("abc"));
        assert_eq!(query_param(Some("x=1&session=s1"), "session"), Some("s1"));
        assert_eq!(query_param(Some("flag&x=1"), "flag"), None);
        assert_eq!(query_param(Some("tokenx=1"), "token"), None);
        assert_eq!(query_param(None, "token"), None);
    }

    #[test]
    fn base64_matches_the_standard_encoding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"\xff\xfe\xfd"), "//79");
    }

    #[test]
    fn the_report_policy_hashes_only_the_scripts_that_run() {
        let html = "<head><style>a{}</style></head><script type=\"application/json\" id=\"data\">{\"a\":1}</script>\
                    <script>let x = 1;</script><script src=\"/x.js\"></script><script>go()</script>";
        let csp = metrics_csp(html);
        // sha256("let x = 1;") and sha256("go()"), computed independently.
        let one = format!("'sha256-{}'", base64(&Sha256::digest(b"let x = 1;")));
        let two = format!("'sha256-{}'", base64(&Sha256::digest(b"go()")));
        assert!(csp.contains(&format!("script-src {one} {two};")), "{csp}");
        assert!(csp.contains("style-src 'unsafe-inline'") && csp.contains("default-src 'none'"));
        assert!(csp.contains("frame-ancestors 'none'") && !csp.contains("connect-src"));
        assert!(metrics_csp("<p>no scripts</p>").contains("script-src 'none';"));
        assert_eq!(inline_scripts("<script>never closed"), Vec::<&str>::new());
        assert_eq!(inline_scripts("<script never"), Vec::<&str>::new());
    }
}
