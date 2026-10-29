//! Whether turning routing and compression on is worth it, from what the proxy measured.
//!
//! Both start in `observe`: they record what they would do and change nothing. This module turns
//! those records into one honest sentence each (for `tokred doctor`, `tokred routing status` and
//! `tokred compression status`) and, when the figures are worth acting on, one line for the user
//! (the savings hint). The estimates are the proxy's own and are called estimates everywhere.

use serde::Serialize;

use crate::config::TrafficMode;
use crate::error::StoreError;
use crate::types::Timestamp;

/// Days of measurements the verdicts look at.
pub const PERIOD_DAYS: i64 = 7;
/// Routing is worth turning on above this estimated saving over the period, in USD.
pub const ROUTING_MIN_USD: f64 = 1.0;
/// Compression is worth turning on above this share of the input tokens.
pub const COMPRESSION_MIN_SHARE: f64 = 0.05;
/// Seconds between two evaluations of the savings hint (a week).
pub const HINT_EVERY_SECS: i64 = 7 * 24 * 60 * 60;

/// What a period of measurements says, before any judgement.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SavingsFigures {
    /// Length of the period, in days.
    pub days: f64,
    /// Routing decisions taken in the period.
    pub routing_decisions: u64,
    /// Requests routing changed (`applied`) or would have changed (`observed`).
    pub routing_changed: u64,
    /// Estimated saving of those requests, in USD.
    pub routing_saving_usd: f64,
    /// Tokens compression would have kept out of the requests (estimate: bytes / 4).
    pub compression_tokens: u64,
    /// Input tokens of the period (new, cache read and cache write).
    pub input_tokens: u64,
}

/// How a feature's figures read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Nothing was measured in the period.
    NoData,
    /// Measured, and too small to bother.
    NotWorthIt,
    /// Measured, and large enough to turn the feature on.
    Worth,
}

/// Where the savings figures come from (the store).
pub trait SavingsRepo {
    /// The figures of `from <= ts < to`.
    fn savings_figures(&self, from: Timestamp, to: Timestamp)
    -> Result<SavingsFigures, StoreError>;
    /// Whether the savings hint is due, and when it is, schedules the next one `every_secs` later.
    /// Of two callers at the same time only one gets `true`.
    fn claim_savings_hint(&self, now: Timestamp, every_secs: i64) -> Result<bool, StoreError>;
}

/// A count as a float (the loss above 2^53 does not matter for an estimate).
#[allow(clippy::cast_precision_loss)]
fn to_f64(count: u64) -> f64 {
    count as f64
}

/// A number of tokens as `1.2M`, `340k` or `87`.
fn compact(tokens: u64) -> String {
    let value = to_f64(tokens);
    if tokens >= 1_000_000 {
        format!("{:.2}M", value / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.0}k", value / 1_000.0)
    } else {
        tokens.to_string()
    }
}

impl SavingsFigures {
    /// Share of the input tokens that compression would have kept out (0 with no input).
    #[must_use]
    pub fn compression_share(&self) -> f64 {
        if self.input_tokens == 0 {
            return 0.0;
        }
        to_f64(self.compression_tokens) / to_f64(self.input_tokens)
    }

    /// The estimated routing saving of a month at the pace of the period.
    #[must_use]
    pub fn routing_per_month(&self) -> f64 {
        self.routing_saving_usd * 30.0 / self.days.max(1.0 / 24.0)
    }

    /// Whether routing is worth turning on.
    #[must_use]
    pub fn routing_verdict(&self) -> Verdict {
        if self.routing_decisions == 0 {
            Verdict::NoData
        } else if self.routing_saving_usd >= ROUTING_MIN_USD {
            Verdict::Worth
        } else {
            Verdict::NotWorthIt
        }
    }

    /// Whether compression is worth turning on.
    #[must_use]
    pub fn compression_verdict(&self) -> Verdict {
        if self.input_tokens == 0 || self.compression_tokens == 0 {
            Verdict::NoData
        } else if self.compression_share() >= COMPRESSION_MIN_SHARE {
            Verdict::Worth
        } else {
            Verdict::NotWorthIt
        }
    }

    /// One sentence on routing, for the period (`days`).
    #[must_use]
    pub fn routing_line(&self) -> String {
        let days = self.days.round();
        match self.routing_verdict() {
            Verdict::NoData => format!("no routing decisions in the last {days:.0} days yet"),
            Verdict::NotWorthIt if self.routing_changed == 0 => format!(
                "none of {} requests in the last {days:.0} days would have been rerouted: not worth turning on yet",
                self.routing_decisions
            ),
            Verdict::NotWorthIt => format!(
                "would have changed {} of {} requests in the last {days:.0} days and saved about ${:.2} (an estimate): not worth turning on yet",
                self.routing_changed, self.routing_decisions, self.routing_saving_usd
            ),
            Verdict::Worth => format!(
                "would have changed {} of {} requests in the last {days:.0} days and saved about ${:.2}, about ${:.2} a month (an estimate): worth turning on (tokred routing on)",
                self.routing_changed,
                self.routing_decisions,
                self.routing_saving_usd,
                self.routing_per_month()
            ),
        }
    }

    /// One sentence on compression, for the period.
    #[must_use]
    pub fn compression_line(&self) -> String {
        let days = self.days.round();
        let share = self.compression_share() * 100.0;
        let tokens = compact(self.compression_tokens);
        match self.compression_verdict() {
            Verdict::NoData => format!("nothing measured in the last {days:.0} days yet"),
            Verdict::NotWorthIt => format!(
                "would have kept about {tokens} tokens ({share:.2}% of the input, an estimate) out of the requests in the last {days:.0} days: not worth turning on yet"
            ),
            Verdict::Worth => format!(
                "would have kept about {tokens} tokens ({share:.1}% of the input, an estimate) out of the requests in the last {days:.0} days: worth turning on (tokred compression on)"
            ),
        }
    }

    /// The one-line message for the user, when turning something on is worth it; `None` when
    /// nothing is (a feature already on, or with figures too small, is never mentioned).
    #[must_use]
    pub fn hint(&self, routing: TrafficMode, compression: TrafficMode) -> Option<String> {
        let mut parts = Vec::new();
        if routing == TrafficMode::Observe && self.routing_verdict() == Verdict::Worth {
            parts.push(format!(
                "routing would have saved about ${:.2} in the last {:.0} days (tokred routing on)",
                self.routing_saving_usd,
                self.days.round()
            ));
        }
        if compression == TrafficMode::Observe && self.compression_verdict() == Verdict::Worth {
            parts.push(format!(
                "compression would have kept about {} tokens ({:.0}% of the input) out of your requests (tokred compression on)",
                compact(self.compression_tokens),
                self.compression_share() * 100.0
            ));
        }
        if parts.is_empty() {
            return None;
        }
        Some(format!(
            "tokred (estimate): {}. Details: tokred doctor. Turn this message off with [suggest] savings_hint = false.",
            parts.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn figures() -> SavingsFigures {
        SavingsFigures {
            days: 7.0,
            ..SavingsFigures::default()
        }
    }

    #[test]
    fn compact_numbers() {
        assert_eq!(compact(87), "87");
        assert_eq!(compact(340_000), "340k");
        assert_eq!(compact(866_995), "867k");
        assert_eq!(compact(1_250_000), "1.25M");
    }

    #[test]
    fn nothing_measured_is_no_data() {
        let none = figures();
        assert_eq!(none.routing_verdict(), Verdict::NoData);
        assert_eq!(none.compression_verdict(), Verdict::NoData);
        assert!((none.compression_share() - 0.0).abs() < f64::EPSILON);
        assert!(none.routing_line().contains("no routing decisions"));
        assert!(none.compression_line().contains("nothing measured"));
        assert_eq!(none.hint(TrafficMode::Observe, TrafficMode::Observe), None);
    }

    #[test]
    fn routing_is_worth_it_above_the_threshold() {
        let small = SavingsFigures {
            routing_decisions: 120,
            routing_changed: 0,
            ..figures()
        };
        assert_eq!(small.routing_verdict(), Verdict::NotWorthIt);
        let text = small.routing_line();
        assert!(text.contains("none of 120 requests"), "{text}");

        let some = SavingsFigures {
            routing_decisions: 10,
            routing_changed: 2,
            routing_saving_usd: 0.4,
            ..figures()
        };
        let text = some.routing_line();
        assert!(text.contains("saved about $0.40"), "{text}");
        assert!(some.routing_line().contains("not worth turning on yet"));

        let big = SavingsFigures {
            routing_decisions: 40,
            routing_changed: 25,
            routing_saving_usd: 3.5,
            ..figures()
        };
        assert_eq!(big.routing_verdict(), Verdict::Worth);
        let text = big.routing_line();
        assert!(text.contains("about $15.00 a month"), "{text}");
        assert!((big.routing_per_month() - 15.0).abs() < 1e-9);
    }

    #[test]
    fn compression_is_judged_by_its_share_of_the_input() {
        let tiny = SavingsFigures {
            compression_tokens: 866_995,
            input_tokens: 1_056_000_000,
            ..figures()
        };
        assert_eq!(tiny.compression_verdict(), Verdict::NotWorthIt);
        let text = tiny.compression_line();
        assert!(text.contains("867k tokens (0.08% of the input"), "{text}");

        let big = SavingsFigures {
            compression_tokens: 12_000_000,
            input_tokens: 100_000_000,
            ..figures()
        };
        assert_eq!(big.compression_verdict(), Verdict::Worth);
        let text = big.compression_line();
        assert!(text.contains("12.00M tokens (12.0% of the input"), "{text}");
    }

    #[test]
    fn the_hint_names_only_what_is_worth_and_not_yet_on() {
        let both = SavingsFigures {
            routing_decisions: 40,
            routing_changed: 25,
            routing_saving_usd: 3.5,
            compression_tokens: 12_000_000,
            input_tokens: 100_000_000,
            ..figures()
        };
        let text = both
            .hint(TrafficMode::Observe, TrafficMode::Observe)
            .unwrap();
        assert!(
            text.contains("routing would have saved about $3.50"),
            "{text}"
        );
        assert!(
            text.contains("compression would have kept about 12.00M tokens (12% of the input)"),
            "{text}"
        );
        assert!(text.contains("savings_hint = false"));

        let only_compression = both
            .hint(TrafficMode::Enforce, TrafficMode::Observe)
            .unwrap();
        assert!(!only_compression.contains("routing"), "{only_compression}");
        assert_eq!(both.hint(TrafficMode::Enforce, TrafficMode::Enforce), None);
        assert_eq!(both.hint(TrafficMode::Off, TrafficMode::Off), None);
    }
}
