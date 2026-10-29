//! Who may open the dashboard. Other local users can reach loopback, and a browser cannot send
//! the admin secret, so the CLI asks for a one-time token (`POST /__tokred/dashboard-token`,
//! with the secret) and the browser trades it for a cookie by opening `/dashboard?token=...`.
//! Tokens live 60 seconds and work once; cookies live in memory until the proxy restarts or
//! they expire. Every comparison runs in time independent of where the values differ.

use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use hyper::HeaderMap;
use hyper::header::COOKIE;

use crate::admin::constant_time_eq;

/// Name of the cookie that carries the browser's session.
pub(crate) const COOKIE_NAME: &str = "tokred_dash";

/// How long a token can be redeemed.
pub(crate) const TOKEN_TTL: Duration = Duration::from_secs(60);
/// How long a browser session lasts.
const SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);
/// Tokens waiting to be redeemed at once; the oldest is dropped beyond this.
const MAX_TOKENS: usize = 16;
/// Browser sessions at once; the oldest is dropped beyond this.
const MAX_SESSIONS: usize = 32;

/// 256 random bits as hex. `RandomState` is seeded from the operating system's randomness, so
/// this needs no extra dependency (the admin secret of the CLI is made the same way).
fn random_hex() -> String {
    (0..4).fold(String::with_capacity(64), |mut out, round| {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(round);
        let _ = write!(out, "{:016x}", hasher.finish());
        out
    })
}

/// Index of the entry equal to `given` and not expired. Every entry is compared, so the time
/// taken does not tell where a guess went wrong.
fn find(entries: &[(String, Instant)], given: &str, now: Instant) -> Option<usize> {
    let mut found = None;
    for (index, (value, expires)) in entries.iter().enumerate() {
        let same = constant_time_eq(given.as_bytes(), value.as_bytes());
        if same && *expires > now {
            found = Some(index);
        }
    }
    found
}

fn push(entries: &mut Vec<(String, Instant)>, value: String, expires: Instant, cap: usize) {
    if entries.len() >= cap {
        entries.remove(0);
    }
    entries.push((value, expires));
}

#[derive(Debug, Default)]
struct State {
    tokens: Vec<(String, Instant)>,
    sessions: Vec<(String, Instant)>,
}

/// One-time tokens and the browser sessions they were traded for.
#[derive(Debug, Default)]
pub(crate) struct DashAuth {
    state: Mutex<State>,
}

impl DashAuth {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new token, valid for [`TOKEN_TTL`] and once.
    pub(crate) fn issue_token(&self, now: Instant) -> String {
        let token = random_hex();
        let mut state = self.lock();
        state.tokens.retain(|(_, expires)| *expires > now);
        push(
            &mut state.tokens,
            token.clone(),
            now + TOKEN_TTL,
            MAX_TOKENS,
        );
        token
    }

    /// Trades a token for a browser session: the cookie value, or `None` when the token is
    /// wrong, used or expired. The token is spent either way it matched.
    pub(crate) fn redeem(&self, token: &str, now: Instant) -> Option<String> {
        let mut state = self.lock();
        let index = find(&state.tokens, token, now)?;
        state.tokens.remove(index);
        state.sessions.retain(|(_, expires)| *expires > now);
        let cookie = random_hex();
        push(
            &mut state.sessions,
            cookie.clone(),
            now + SESSION_TTL,
            MAX_SESSIONS,
        );
        Some(cookie)
    }

    /// Whether a cookie value is a live browser session.
    pub(crate) fn session_valid(&self, cookie: &str, now: Instant) -> bool {
        find(&self.lock().sessions, cookie, now).is_some()
    }

    /// Forgets what expired.
    pub(crate) fn sweep(&self, now: Instant) {
        let mut state = self.lock();
        state.tokens.retain(|(_, expires)| *expires > now);
        state.sessions.retain(|(_, expires)| *expires > now);
    }
}

/// The value of the dashboard cookie in a request, if it sent one.
pub(crate) fn cookie_value(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|line| line.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(name, value)| (name == COOKIE_NAME).then_some(value))
}

/// The `Set-Cookie` value that stores a browser session: unreadable by scripts, never sent
/// from another site, for the whole proxy origin (`/metrics` shares it with `/dashboard`).
pub(crate) fn set_cookie(value: &str) -> String {
    format!("{COOKIE_NAME}={value}; HttpOnly; SameSite=Strict; Path=/")
}

#[cfg(test)]
mod tests {
    use hyper::header::HeaderValue;

    use super::*;

    #[test]
    fn random_values_are_long_and_different() {
        let (a, b) = (random_hex(), random_hex());
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn a_token_works_once() {
        let auth = DashAuth::default();
        let now = Instant::now();
        let token = auth.issue_token(now);
        assert!(auth.redeem("not the token", now).is_none());
        assert!(auth.redeem("", now).is_none());
        let cookie = auth.redeem(&token, now).unwrap();
        assert!(auth.session_valid(&cookie, now));
        assert!(auth.redeem(&token, now).is_none(), "spent");
        assert!(
            auth.session_valid(&cookie, now),
            "the session outlives the token"
        );
    }

    #[test]
    fn a_token_expires_after_sixty_seconds() {
        let auth = DashAuth::default();
        let now = Instant::now();
        let token = auth.issue_token(now);
        assert!(auth.redeem(&token, now + TOKEN_TTL).is_none());
        let again = auth.issue_token(now);
        assert!(auth.redeem(&again, now + Duration::from_secs(59)).is_some());
    }

    #[test]
    fn a_session_cookie_must_match_exactly_and_expires() {
        let auth = DashAuth::default();
        let now = Instant::now();
        let cookie = auth.redeem(&auth.issue_token(now), now).unwrap();
        assert!(!auth.session_valid("", now));
        assert!(!auth.session_valid(&cookie[..63], now));
        assert!(!auth.session_valid(&format!("{cookie}0"), now));
        let mut wrong = cookie.clone();
        wrong.replace_range(63.., if cookie.ends_with('0') { "1" } else { "0" });
        assert!(!auth.session_valid(&wrong, now));
        assert!(!auth.session_valid(&cookie, now + SESSION_TTL));
        auth.sweep(now + SESSION_TTL);
        assert!(!auth.session_valid(&cookie, now));
    }

    #[test]
    fn the_oldest_token_and_session_are_dropped_when_there_are_too_many() {
        let auth = DashAuth::default();
        let now = Instant::now();
        let tokens: Vec<String> = (0..=MAX_TOKENS).map(|_| auth.issue_token(now)).collect();
        assert!(
            auth.redeem(&tokens[0], now).is_none(),
            "the oldest was dropped"
        );
        let cookies: Vec<String> = tokens[1..]
            .iter()
            .map(|token| auth.redeem(token, now).unwrap())
            .collect();
        assert_eq!(cookies.len(), MAX_TOKENS);
        let more: Vec<String> = (0..=MAX_SESSIONS - MAX_TOKENS)
            .map(|_| auth.redeem(&auth.issue_token(now), now).unwrap())
            .collect();
        assert!(
            !auth.session_valid(&cookies[0], now),
            "the oldest session was dropped"
        );
        assert!(auth.session_valid(more.last().unwrap(), now));
    }

    #[test]
    fn sweeping_forgets_expired_tokens() {
        let auth = DashAuth::default();
        let now = Instant::now();
        let token = auth.issue_token(now);
        auth.sweep(now + TOKEN_TTL + Duration::from_secs(1));
        assert!(auth.redeem(&token, now).is_none());
    }

    #[test]
    fn the_cookie_is_found_among_others_and_set_with_its_protections() {
        let mut headers = HeaderMap::new();
        assert_eq!(cookie_value(&headers), None);
        headers.append(
            COOKIE,
            HeaderValue::from_static("a=1; tokred_dash=abc123; b=2"),
        );
        assert_eq!(cookie_value(&headers), Some("abc123"));
        let mut other = HeaderMap::new();
        other.append(COOKIE, HeaderValue::from_static("session=x"));
        other.append(
            COOKIE,
            HeaderValue::from_static("tokred_dashboard=nope; flag"),
        );
        assert_eq!(cookie_value(&other), None);
        other.append(COOKIE, HeaderValue::from_bytes(b"\xff").unwrap());
        assert_eq!(cookie_value(&other), None);
        let cookie = set_cookie("v");
        assert_eq!(cookie, "tokred_dash=v; HttpOnly; SameSite=Strict; Path=/");
    }
}
