//! Time spans (`7d`, `12h`, `2w`) and half-open time ranges.

use serde::{Serialize, Serializer};
use tokred_core::Timestamp;

use crate::util::SECS_PER_DAY;

/// The text is not a valid span.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "invalid span `{0}`: use a positive whole number followed by h, d or w (for example 12h, 7d, 2w)"
)]
pub struct SpanError(pub String);

/// Parses `"<n>h"`, `"<n>d"` or `"<n>w"` (n >= 1) into seconds.
pub fn parse_span(input: &str) -> Result<i64, SpanError> {
    parse(input.trim()).ok_or_else(|| SpanError(input.to_owned()))
}

fn parse(text: &str) -> Option<i64> {
    let multiplier = match text.chars().next_back()? {
        'h' => 3_600,
        'd' => SECS_PER_DAY,
        'w' => 7 * SECS_PER_DAY,
        _ => return None,
    };
    // The unit is one ASCII byte, so this slice is always on a character boundary.
    let digits = &text[..text.len() - 1];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let count: i64 = digits.parse().ok()?;
    if count == 0 {
        return None;
    }
    count.checked_mul(multiplier)
}

/// A half-open time range `[from, to)` in unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    /// First second included.
    pub from: Timestamp,
    /// First second excluded.
    pub to: Timestamp,
}

impl TimeRange {
    /// Builds a range.
    #[must_use]
    pub const fn new(from: Timestamp, to: Timestamp) -> Self {
        Self { from, to }
    }

    /// The last `span_secs` seconds up to and including `now`: `[now - span, now + 1)`.
    #[must_use]
    pub fn last(now: Timestamp, span_secs: i64) -> Self {
        Self {
            from: Timestamp(now.0.saturating_sub(span_secs)),
            to: Timestamp(now.0.saturating_add(1)),
        }
    }

    /// Every timestamp.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            from: Timestamp(i64::MIN),
            to: Timestamp(i64::MAX),
        }
    }

    /// Whether `ts` falls inside the range.
    #[must_use]
    pub fn contains(&self, ts: Timestamp) -> bool {
        self.from <= ts && ts < self.to
    }
}

impl Serialize for TimeRange {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Raw {
            from: i64,
            to: i64,
        }
        Raw {
            from: self.from.0,
            to: self.to.0,
        }
        .serialize(serializer)
    }
}

/// Turns a span such as `"7d"` into the range ending at `now`.
pub fn range_from_span(now: Timestamp, span: &str) -> Result<TimeRange, SpanError> {
    parse_span(span).map(|secs| TimeRange::last(now, secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_parse_to_seconds() {
        assert_eq!(parse_span("12h"), Ok(43_200));
        assert_eq!(parse_span("7d"), Ok(604_800));
        assert_eq!(parse_span("2w"), Ok(1_209_600));
        assert_eq!(parse_span("30d"), Ok(2_592_000));
        assert_eq!(parse_span(" 1d "), Ok(86_400));
    }

    #[test]
    fn bad_spans_are_rejected() {
        for bad in [
            "",
            "d",
            "7",
            "0d",
            "-1d",
            "+1d",
            "1.5d",
            "7x",
            "7 d",
            "é",
            "1é",
            "99999999999999999999d",
            "9223372036854775807w",
        ] {
            let err = parse_span(bad).unwrap_err();
            assert_eq!(err.0, bad);
            assert!(err.to_string().contains("invalid span"));
        }
    }

    #[test]
    fn ranges_are_half_open_and_include_now() {
        let range = range_from_span(Timestamp(1_000_000), "1d").unwrap();
        assert_eq!(range.from, Timestamp(1_000_000 - 86_400));
        assert!(range.contains(Timestamp(1_000_000)));
        assert!(range.contains(range.from));
        assert!(!range.contains(Timestamp(1_000_001)));
        assert!(!range.contains(Timestamp(range.from.0 - 1)));
        assert!(range_from_span(Timestamp(0), "nope").is_err());
    }

    #[test]
    fn all_contains_everything_and_ranges_serialize() {
        assert!(TimeRange::all().contains(Timestamp(0)));
        assert!(TimeRange::all().contains(Timestamp(i64::MIN)));
        let json = serde_json::to_string(&TimeRange::new(Timestamp(1), Timestamp(2))).unwrap();
        assert_eq!(json, r#"{"from":1,"to":2}"#);
        let saturated = TimeRange::last(Timestamp(i64::MAX), 10);
        assert_eq!(saturated.to, Timestamp(i64::MAX));
    }
}
