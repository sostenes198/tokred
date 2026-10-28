//! Response cache in shadow mode (spec 6.6): measures how many requests exactly repeat one seen
//! in the last 24 hours, by the blake3 of the body's canonical JSON. Only hashes and counts are
//! kept, in memory; nothing is ever altered.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;
use tokred_core::Timestamp;
use tokred_core::compression::{SHADOW_KIND_RESPONSE_CACHE, ShadowRecord};
use tokred_core::proxy::{HookFailure, ProxyHook, ProxyRecord, RecordSink, RequestCtx};

use crate::preview::{RULES_VERSION, estimated_tokens};

/// A request is a repeat when the same body was seen at most this long before.
pub const REPEAT_WINDOW_SECS: i64 = 86_400;

/// Request hashes kept in memory.
pub const MAX_TRACKED_REQUESTS: usize = 4096;

/// The canonical JSON of a value: object keys sorted (recursively), no whitespace. Two bodies
/// that differ only in key order have the same canonical JSON.
#[must_use]
pub fn canonical_json(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            out.push('{');
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical(value, out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

/// blake3 (hex) of the canonical JSON of `body`, and the canonical JSON's length in bytes.
#[must_use]
pub fn canonical_hash(body: &Value) -> (String, usize) {
    let canonical = canonical_json(body);
    (
        blake3::hash(canonical.as_bytes()).to_hex().to_string(),
        canonical.len(),
    )
}

/// Last time each request hash was seen, bounded to [`MAX_TRACKED_REQUESTS`].
#[derive(Debug, Default)]
struct RepeatWindow {
    last_seen: HashMap<String, Timestamp>,
}

impl RepeatWindow {
    /// Notes `hash` at `now`; true when it was seen within the window.
    fn observe(&mut self, hash: &str, now: Timestamp) -> bool {
        let repeat = self
            .last_seen
            .get(hash)
            .is_some_and(|seen| now.0.saturating_sub(seen.0) <= REPEAT_WINDOW_SECS);
        if !self.last_seen.contains_key(hash) && self.last_seen.len() >= MAX_TRACKED_REQUESTS {
            self.last_seen
                .retain(|_, seen| now.0.saturating_sub(seen.0) <= REPEAT_WINDOW_SECS);
            if self.last_seen.len() >= MAX_TRACKED_REQUESTS {
                let oldest = self
                    .last_seen
                    .iter()
                    .min_by_key(|(_, seen)| **seen)
                    .map(|(hash, _)| hash.clone());
                if let Some(oldest) = oldest {
                    self.last_seen.remove(&oldest);
                }
            }
        }
        self.last_seen.insert(hash.to_owned(), now);
        repeat
    }
}

/// The response cache shadow hook. Run it **before** any hook that changes the body, so it
/// measures what Claude Code sent.
pub struct ResponseCacheShadow {
    enabled: bool,
    sink: Arc<dyn RecordSink>,
    window: Mutex<RepeatWindow>,
}

impl std::fmt::Debug for ResponseCacheShadow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResponseCacheShadow")
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

impl ResponseCacheShadow {
    /// A hook that measures repeats when `enabled` (`[shadow] response_cache`).
    #[must_use]
    pub fn new(enabled: bool, sink: Arc<dyn RecordSink>) -> Self {
        Self {
            enabled,
            sink,
            window: Mutex::new(RepeatWindow::default()),
        }
    }
}

impl ProxyHook for ResponseCacheShadow {
    fn name(&self) -> &'static str {
        "response-cache-shadow"
    }

    fn before(&self, ctx: &mut RequestCtx<'_>) -> Result<(), HookFailure> {
        if !self.enabled {
            return Ok(());
        }
        let (hash, bytes) = canonical_hash(&ctx.body);
        let repeat = self
            .window
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .observe(&hash, ctx.now);
        if repeat {
            self.sink.record(ProxyRecord::Shadow(Box::new(ShadowRecord {
                ts: ctx.now,
                session_id: ctx.meta.session_id.clone().unwrap_or_default(),
                kind: SHADOW_KIND_RESPONSE_CACHE.to_owned(),
                key: hash,
                bytes_saved: u64::try_from(bytes).unwrap_or(u64::MAX),
                est_tokens_saved: estimated_tokens(bytes),
                rules_version: RULES_VERSION.to_owned(),
            })));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn canonical_json_sorts_keys_recursively() {
        let a = json!({"b": [1, {"y": "\"q\"", "x": null}], "a": true, "c": 1.5});
        let b = json!({"c": 1.5, "a": true, "b": [1, {"x": null, "y": "\"q\""}]});
        assert_eq!(
            canonical_json(&a),
            r#"{"a":true,"b":[1,{"x":null,"y":"\"q\""}],"c":1.5}"#
        );
        assert_eq!(canonical_hash(&a), canonical_hash(&b));
        assert_ne!(canonical_hash(&a).0, canonical_hash(&json!({})).0);
        assert_eq!(canonical_json(&json!([])), "[]");
    }

    #[test]
    fn repeats_count_within_the_window_only() {
        let mut window = RepeatWindow::default();
        assert!(!window.observe("h", Timestamp(0)));
        assert!(window.observe("h", Timestamp(REPEAT_WINDOW_SECS)));
        // Sliding: the window counts from the last sighting.
        assert!(window.observe("h", Timestamp(2 * REPEAT_WINDOW_SECS)));
        assert!(!window.observe("h", Timestamp(3 * REPEAT_WINDOW_SECS + 1)));
    }

    #[test]
    fn the_window_is_bounded() {
        let mut window = RepeatWindow::default();
        for n in 0..MAX_TRACKED_REQUESTS {
            window.observe(&format!("old{n}"), Timestamp(0));
        }
        // Full of stale entries: they are dropped.
        assert!(!window.observe("fresh", Timestamp(REPEAT_WINDOW_SECS + 1)));
        assert_eq!(window.last_seen.len(), 1);

        let mut window = RepeatWindow::default();
        for n in 0..MAX_TRACKED_REQUESTS {
            window.observe(&format!("h{n}"), Timestamp(i64::try_from(n).unwrap() + 1));
        }
        // Full of fresh entries: the oldest one goes.
        assert!(!window.observe("new", Timestamp(10_000)));
        assert_eq!(window.last_seen.len(), MAX_TRACKED_REQUESTS);
        assert!(!window.last_seen.contains_key("h0"));
        assert!(window.last_seen.contains_key("h1"));
        // A known hash never evicts anything.
        assert!(window.observe("h1", Timestamp(10_001)));
        assert_eq!(window.last_seen.len(), MAX_TRACKED_REQUESTS);
    }
}
