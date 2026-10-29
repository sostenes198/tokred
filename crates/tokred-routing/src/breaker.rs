//! Circuit breaker: when too many of the last [`WINDOW`] rewrites were rejected by
//! the API, rewriting stops for the cooldown.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use tokred_core::Timestamp;

/// Number of most recent rewrites the failure rate is computed over.
pub const WINDOW: usize = 50;

/// Cooldown used when the configured value cannot be read.
const DEFAULT_COOLDOWN_SECS: i64 = 30 * 60;

/// Parses a duration such as `30m`, `90s`, `6h` or `7d` into seconds. A bare number is
/// seconds. `None` for anything else.
#[must_use]
pub fn parse_duration(text: &str) -> Option<i64> {
    let text = text.trim();
    let (number, unit) = match text.char_indices().last()? {
        (index, unit) if unit.is_ascii_alphabetic() => (&text[..index], unit),
        _ => (text, 's'),
    };
    let multiplier = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        _ => return None,
    };
    number
        .parse::<i64>()
        .ok()
        .filter(|n| *n >= 0)
        .and_then(|n| n.checked_mul(multiplier))
}

/// What `tokred routing status` shows about the breaker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreakerStatus {
    /// Rewriting is stopped.
    pub open: bool,
    /// Until when (seconds since the epoch), when open.
    pub open_until: Option<i64>,
    /// Rewrites in the window (at most [`WINDOW`]).
    pub rewrites: usize,
    /// Of those, how many the API rejected.
    pub failures: usize,
    /// `failures / WINDOW`.
    pub failure_rate: f64,
    /// The configured limit.
    pub max_failure_rate: f64,
    /// The cooldown, in seconds.
    pub cooldown_secs: i64,
}

/// The breaker's state.
#[derive(Debug, Clone)]
pub(crate) struct Breaker {
    /// One entry per rewrite, `true` when it was rejected; newest last.
    window: VecDeque<bool>,
    open_until: Option<Timestamp>,
    max_failure_rate: f64,
    cooldown_secs: i64,
}

impl Breaker {
    pub(crate) fn new(max_failure_rate: f64, cooldown: &str) -> Self {
        Self {
            window: VecDeque::with_capacity(WINDOW),
            open_until: None,
            max_failure_rate,
            cooldown_secs: parse_duration(cooldown).unwrap_or(DEFAULT_COOLDOWN_SECS),
        }
    }

    /// Whether rewriting is stopped at `now`. A cooldown that has ended closes the breaker
    /// and starts a fresh window.
    pub(crate) fn is_open(&mut self, now: Timestamp) -> bool {
        match self.open_until {
            Some(until) if now < until => true,
            Some(_) => {
                self.open_until = None;
                self.window.clear();
                false
            }
            None => false,
        }
    }

    /// A request was rewritten.
    pub(crate) fn record_rewrite(&mut self) {
        self.push(false);
    }

    fn push(&mut self, failed: bool) {
        if self.window.len() == WINDOW {
            self.window.pop_front();
        }
        self.window.push_back(failed);
    }

    /// The API rejected a rewritten request. Opens the breaker when the failures in the
    /// window exceed the configured rate.
    pub(crate) fn record_failure(&mut self, now: Timestamp) {
        if let Some(entry) = self.window.iter_mut().rev().find(|failed| !**failed) {
            *entry = true;
        } else {
            self.push(true);
        }
        if self.open_until.is_none() && self.rate() > self.max_failure_rate {
            self.open_until = Some(Timestamp(now.0.saturating_add(self.cooldown_secs)));
        }
    }

    fn failures(&self) -> usize {
        self.window.iter().filter(|failed| **failed).count()
    }

    #[allow(clippy::cast_precision_loss)]
    fn rate(&self) -> f64 {
        self.failures() as f64 / WINDOW as f64
    }

    pub(crate) fn status(&mut self, now: Timestamp) -> BreakerStatus {
        let open = self.is_open(now);
        BreakerStatus {
            open,
            open_until: self.open_until.map(|t| t.0),
            rewrites: self.window.len(),
            failures: self.failures(),
            failure_rate: self.rate(),
            max_failure_rate: self.max_failure_rate,
            cooldown_secs: self.cooldown_secs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_parse() {
        assert_eq!(parse_duration("30m"), Some(1800));
        assert_eq!(parse_duration(" 90s "), Some(90));
        assert_eq!(parse_duration("6h"), Some(21_600));
        assert_eq!(parse_duration("7d"), Some(604_800));
        assert_eq!(parse_duration("45"), Some(45));
        for bad in ["", "m", "3w", "-5m", "x5m", "99999999999999999999d"] {
            assert_eq!(parse_duration(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_bad_cooldown_falls_back_to_thirty_minutes() {
        assert_eq!(Breaker::new(0.05, "soon").cooldown_secs, 1800);
    }

    #[test]
    fn it_opens_above_the_rate_and_closes_after_the_cooldown() {
        let mut breaker = Breaker::new(0.05, "30m");
        let t0 = Timestamp(1_000);
        for _ in 0..60 {
            breaker.record_rewrite();
        }
        assert_eq!(breaker.status(t0).rewrites, WINDOW);
        // 2 of 50 is 4%: still closed. The third failure (6%) opens it.
        breaker.record_failure(t0);
        breaker.record_failure(t0);
        assert!(!breaker.is_open(t0));
        breaker.record_failure(Timestamp(1_010));
        let status = breaker.status(Timestamp(1_011));
        assert!(status.open);
        assert_eq!(status.open_until, Some(1_010 + 1800));
        assert_eq!(status.failures, 3);
        assert!((status.failure_rate - 0.06).abs() < 1e-9);
        // A failure while open does not extend the cooldown.
        breaker.record_failure(Timestamp(1_500));
        assert_eq!(breaker.status(Timestamp(1_500)).open_until, Some(2_810));
        // Closed again once the cooldown is over, with a fresh window.
        let closed = breaker.status(Timestamp(2_810));
        assert!(!closed.open);
        assert_eq!((closed.rewrites, closed.failures), (0, 0));
    }

    #[test]
    fn a_failure_without_a_recorded_rewrite_still_counts() {
        let mut breaker = Breaker::new(0.0, "1m");
        breaker.record_failure(Timestamp(5));
        let status = breaker.status(Timestamp(5));
        assert!(status.open);
        assert_eq!((status.rewrites, status.failures), (1, 1));
        assert!((status.max_failure_rate).abs() < 1e-9);
        assert_eq!(status.cooldown_secs, 60);
    }

    #[test]
    fn old_failures_leave_the_window() {
        let mut breaker = Breaker::new(0.05, "30m");
        breaker.record_rewrite();
        breaker.record_failure(Timestamp(0));
        for _ in 0..WINDOW {
            breaker.record_rewrite();
        }
        assert_eq!(breaker.status(Timestamp(0)).failures, 0);
    }
}
