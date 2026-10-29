//! Who may open the dashboard. Other local users can reach loopback, and a browser cannot send
//! the admin secret, so the CLI asks for a one-time token (`POST /__tokred/dashboard-token`,
//! with the secret) and the browser trades it for a cookie by opening `/dashboard?token=...`.
//! Tokens live 60 seconds and work once and are kept in memory. The cookie is signed with a key
//! derived from the admin secret and carries its own expiry, so it stays valid for 30 days and
//! across proxy restarts, and stops working when the secret is replaced. Every comparison runs in
//! time independent of where the values differ.

use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use hyper::HeaderMap;
use hyper::header::COOKIE;
use sha2::{Digest, Sha256};

use crate::admin::constant_time_eq;

/// Name of the cookie that carries the browser's session.
pub(crate) const COOKIE_NAME: &str = "tokred_dash";

/// How long a token can be redeemed.
pub(crate) const TOKEN_TTL: Duration = Duration::from_secs(60);
/// How long a browser session lasts, in seconds (30 days).
const SESSION_TTL_SECS: i64 = 30 * 24 * 60 * 60;
/// Tokens waiting to be redeemed at once; the oldest is dropped beyond this.
const MAX_TOKENS: usize = 16;
/// Tells a session key from the other uses of the admin secret.
const KEY_LABEL: &[u8] = b"tokred dashboard session";
/// What a session cookie starts with (its format version).
const COOKIE_VERSION: &str = "v1";

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

/// HMAC-SHA256 (RFC 2104).
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut padded = [0_u8; BLOCK];
    if key.len() > BLOCK {
        padded[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        padded[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(padded.map(|byte| byte ^ 0x36));
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(padded.map(|byte| byte ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Index of the token equal to `given` and not expired. Every entry is compared, so the time
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

/// One-time tokens, and the signed cookies they are traded for.
#[derive(Debug)]
pub(crate) struct DashAuth {
    /// The key cookies are signed with; `None` without an admin secret (nothing is signed with
    /// a key anybody could guess).
    key: Option<[u8; 32]>,
    tokens: Mutex<Vec<(String, Instant)>>,
}

impl DashAuth {
    /// Signs cookies with a key derived from the admin `secret`.
    pub(crate) fn new(secret: &str) -> Self {
        Self {
            key: (!secret.is_empty()).then(|| hmac_sha256(secret.as_bytes(), KEY_LABEL)),
            tokens: Mutex::new(Vec::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<(String, Instant)>> {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A new token, valid for [`TOKEN_TTL`] and once.
    pub(crate) fn issue_token(&self, now: Instant) -> String {
        let token = random_hex();
        let mut tokens = self.lock();
        tokens.retain(|(_, expires)| *expires > now);
        push(&mut tokens, token.clone(), now + TOKEN_TTL, MAX_TOKENS);
        token
    }

    /// Trades a token for a browser session: the cookie value, or `None` when the token is
    /// wrong, used or expired (or there is no key to sign with). The token is spent when it
    /// matched.
    pub(crate) fn redeem(&self, token: &str, now: Instant, unix_now: i64) -> Option<String> {
        let mut tokens = self.lock();
        let index = find(&tokens, token, now)?;
        tokens.remove(index);
        let key = self.key?;
        let body = format!(
            "{COOKIE_VERSION}.{}",
            unix_now.saturating_add(SESSION_TTL_SECS)
        );
        let mac = hex(&hmac_sha256(&key, body.as_bytes()));
        Some(format!("{body}.{mac}"))
    }

    /// Whether a cookie value is a session signed with this key that has not expired.
    pub(crate) fn session_valid(&self, cookie: &str, unix_now: i64) -> bool {
        let Some(key) = self.key else {
            return false;
        };
        let Some((body, mac)) = cookie.rsplit_once('.') else {
            return false;
        };
        let wanted = hex(&hmac_sha256(&key, body.as_bytes()));
        let genuine = constant_time_eq(mac.as_bytes(), wanted.as_bytes());
        let expiry = body
            .strip_prefix(COOKIE_VERSION)
            .and_then(|rest| rest.strip_prefix('.'))
            .and_then(|seconds| seconds.parse::<i64>().ok());
        genuine && expiry.is_some_and(|expires| expires > unix_now)
    }

    /// Forgets the tokens that expired.
    pub(crate) fn sweep(&self, now: Instant) {
        self.lock().retain(|(_, expires)| *expires > now);
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
/// from another site, for the whole proxy origin (`/metrics` shares it with `/dashboard`), kept
/// by the browser for as long as the session lasts.
pub(crate) fn set_cookie(value: &str) -> String {
    format!("{COOKIE_NAME}={value}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_TTL_SECS}")
}

#[cfg(test)]
mod tests {
    use hyper::header::HeaderValue;

    use super::*;

    const SECRET: &str = "a-long-enough-admin-secret-for-the-tests";
    const NOW: i64 = 1_800_000_000;

    fn signed_in(auth: &DashAuth, at: Instant) -> String {
        auth.redeem(&auth.issue_token(at), at, NOW).unwrap()
    }

    #[test]
    fn random_values_are_long_and_different() {
        let (a, b) = (random_hex(), random_hex());
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn hmac_matches_the_rfc_4231_vectors() {
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // A key longer than the block is hashed first (test case 6).
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn a_token_works_once() {
        let auth = DashAuth::new(SECRET);
        let now = Instant::now();
        let token = auth.issue_token(now);
        assert!(auth.redeem("not the token", now, NOW).is_none());
        assert!(auth.redeem("", now, NOW).is_none());
        let cookie = auth.redeem(&token, now, NOW).unwrap();
        assert!(auth.session_valid(&cookie, NOW));
        assert!(auth.redeem(&token, now, NOW).is_none(), "spent");
        assert!(
            auth.session_valid(&cookie, NOW),
            "the session outlives the token"
        );
    }

    #[test]
    fn a_token_expires_after_sixty_seconds() {
        let auth = DashAuth::new(SECRET);
        let now = Instant::now();
        let token = auth.issue_token(now);
        assert!(auth.redeem(&token, now + TOKEN_TTL, NOW).is_none());
        let again = auth.issue_token(now);
        assert!(
            auth.redeem(&again, now + Duration::from_secs(59), NOW)
                .is_some()
        );
    }

    #[test]
    fn a_session_lasts_thirty_days() {
        let auth = DashAuth::new(SECRET);
        let cookie = signed_in(&auth, Instant::now());
        assert!(cookie.starts_with(&format!("v1.{}.", NOW + SESSION_TTL_SECS)));
        assert!(auth.session_valid(&cookie, NOW + SESSION_TTL_SECS - 1));
        assert!(!auth.session_valid(&cookie, NOW + SESSION_TTL_SECS));
        assert!(!auth.session_valid(&cookie, NOW + SESSION_TTL_SECS + 1));
    }

    #[test]
    fn a_session_survives_a_restart_but_not_another_secret() {
        let before = signed_in(&DashAuth::new(SECRET), Instant::now());
        assert!(
            DashAuth::new(SECRET).session_valid(&before, NOW),
            "a new proxy with the same secret accepts it"
        );
        assert!(
            !DashAuth::new("another-admin-secret-entirely").session_valid(&before, NOW),
            "a replaced secret signs every browser out"
        );
    }

    #[test]
    fn a_tampered_or_malformed_cookie_is_refused() {
        let auth = DashAuth::new(SECRET);
        let cookie = signed_in(&auth, Instant::now());
        let (body, mac) = cookie.rsplit_once('.').unwrap();
        let expiry = body.strip_prefix("v1.").unwrap();
        let later = format!("v1.{}", expiry.parse::<i64>().unwrap() + 1);
        assert!(
            !auth.session_valid(&format!("{later}.{mac}"), NOW),
            "longer expiry"
        );
        assert!(
            !auth.session_valid(&format!("v2.{expiry}.{mac}"), NOW),
            "other version"
        );
        let mut wrong = mac.to_owned();
        wrong.replace_range(63.., if mac.ends_with('0') { "1" } else { "0" });
        assert!(
            !auth.session_valid(&format!("{body}.{wrong}"), NOW),
            "wrong mac"
        );
        assert!(
            !auth.session_valid(&format!("{body}.{}", &mac[..63]), NOW),
            "short mac"
        );
        assert!(!auth.session_valid(&format!("{cookie}0"), NOW));
        assert!(!auth.session_valid("", NOW));
        assert!(!auth.session_valid("no-dots", NOW));
        // A well signed value whose expiry is not a number.
        let junk = "v1.soon";
        let signed = format!(
            "{junk}.{}",
            hex(&hmac_sha256(&auth.key.unwrap(), junk.as_bytes()))
        );
        assert!(!auth.session_valid(&signed, NOW));
    }

    #[test]
    fn without_a_secret_nothing_is_signed_or_accepted() {
        let auth = DashAuth::new("");
        let now = Instant::now();
        let token = auth.issue_token(now);
        assert!(auth.redeem(&token, now, NOW).is_none());
        let guessable = format!("v1.{}", NOW + 1);
        let forged = format!(
            "{guessable}.{}",
            hex(&hmac_sha256(b"", guessable.as_bytes()))
        );
        assert!(!auth.session_valid(&forged, NOW));
    }

    #[test]
    fn the_oldest_token_is_dropped_when_there_are_too_many() {
        let auth = DashAuth::new(SECRET);
        let now = Instant::now();
        let tokens: Vec<String> = (0..=MAX_TOKENS).map(|_| auth.issue_token(now)).collect();
        assert!(
            auth.redeem(&tokens[0], now, NOW).is_none(),
            "the oldest was dropped"
        );
        assert!(auth.redeem(&tokens[1], now, NOW).is_some());
    }

    #[test]
    fn sweeping_forgets_expired_tokens() {
        let auth = DashAuth::new(SECRET);
        let now = Instant::now();
        let token = auth.issue_token(now);
        auth.sweep(now + TOKEN_TTL + Duration::from_secs(1));
        assert!(auth.redeem(&token, now, NOW).is_none());
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
        assert_eq!(
            cookie,
            "tokred_dash=v; HttpOnly; SameSite=Strict; Path=/; Max-Age=2592000"
        );
    }
}
