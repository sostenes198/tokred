//! Pieces of a [`ProxyRequestRecord`](tokred_core::proxy::ProxyRequestRecord): segment hashes,
//! the transcript message key and request ids. Only hashes and counts, never contents.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Writes `value` as JSON with object keys sorted, so the same content always hashes the same
/// whatever the key order it arrived in.
fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
            out.push('{');
            for (index, (key, item)) in entries.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::from(key.as_str()).to_string());
                out.push(':');
                write_canonical(item, out);
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

/// blake3 (hex) of the canonical JSON of a block; an absent block hashes as `null`.
pub(crate) fn canonical_hash(value: Option<&Value>) -> String {
    let mut out = String::new();
    write_canonical(value.unwrap_or(&Value::Null), &mut out);
    blake3::hash(out.as_bytes()).to_hex().to_string()
}

/// Hash of what decides the thinking part of the cache key: the `thinking` configuration and
/// `output_config.effort` (both invalidate the cache when they change).
pub(crate) fn thinking_signature(body: &Value) -> String {
    let signature = serde_json::json!({
        "thinking": body.get("thinking"),
        "effort": body.pointer("/output_config/effort"),
    });
    canonical_hash(Some(&signature))
}

/// `"<message.id>:<request-id>"`, the key transcripts use (empty request id when absent).
pub(crate) fn message_key(message_id: Option<&str>, request_id: Option<&str>) -> Option<String> {
    message_id.map(|id| format!("{id}:{}", request_id.unwrap_or_default()))
}

/// Whole milliseconds, saturating.
pub(crate) fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Whole microseconds, saturating.
pub(crate) fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// A unique id for a request: 16 hex digits of blake3 over the wall clock, the process id and a
/// per-process counter.
pub(crate) fn new_request_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let seed = format!("{nanos}:{}:{counter}", std::process::id());
    blake3::hash(seed.as_bytes()).to_hex()[..16].to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn canonical_hashes_ignore_key_order_but_not_content() {
        let a = json!({"b": [1, {"y": "é", "x": null}], "a": true, "c": 1.5});
        let b: Value =
            serde_json::from_str(r#"{"c":1.5,"a":true,"b":[1,{"x":null,"y":"\u00e9"}]}"#).unwrap();
        assert_eq!(canonical_hash(Some(&a)), canonical_hash(Some(&b)));
        assert_ne!(
            canonical_hash(Some(&a)),
            canonical_hash(Some(&json!({"a": true})))
        );
        assert_eq!(canonical_hash(None), canonical_hash(Some(&Value::Null)));
        assert_eq!(canonical_hash(None).len(), 64);

        let mut out = String::new();
        write_canonical(&json!({"k\"": ["a", 2]}), &mut out);
        assert_eq!(out, r#"{"k\"":["a",2]}"#);
    }

    #[test]
    fn the_thinking_signature_follows_thinking_and_effort_only() {
        let base = json!({"thinking": {"type": "adaptive"}, "output_config": {"effort": "high"}, "model": "a"});
        let other_model = json!({"thinking": {"type": "adaptive"}, "output_config": {"effort": "high"}, "model": "b"});
        let other_effort =
            json!({"thinking": {"type": "adaptive"}, "output_config": {"effort": "low"}});
        assert_eq!(thinking_signature(&base), thinking_signature(&other_model));
        assert_ne!(thinking_signature(&base), thinking_signature(&other_effort));
        assert_ne!(thinking_signature(&base), thinking_signature(&json!({})));
    }

    #[test]
    fn message_keys_match_the_transcript_format() {
        assert_eq!(
            message_key(Some("msg_1"), Some("req_1")).as_deref(),
            Some("msg_1:req_1")
        );
        assert_eq!(message_key(Some("m"), None).as_deref(), Some("m:"));
        assert_eq!(message_key(None, Some("req")), None);
    }

    #[test]
    fn durations_saturate() {
        assert_eq!(millis(Duration::from_millis(1500)), 1500);
        assert_eq!(micros(Duration::from_micros(42)), 42);
        assert_eq!(millis(Duration::MAX), u64::MAX);
        assert_eq!(micros(Duration::MAX), u64::MAX);
    }

    #[test]
    fn request_ids_are_unique() {
        let ids: std::collections::HashSet<String> = (0..1000).map(|_| new_request_id()).collect();
        assert_eq!(ids.len(), 1000);
        assert!(ids.iter().all(|id| id.len() == 16));
    }
}
