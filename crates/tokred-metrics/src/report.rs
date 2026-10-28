//! Routing and guard summaries for the report, derived from `hook_events`.
//!
//! Conventions (see `tokred_core::hook_event`):
//!
//! * `route` rows hold the task class in `decision` (`none` when the hook was off).
//! * `enforce` rows say `agent_matches` or `agent_mismatch` in `reason`, and what they did in
//!   `decision`.
//! * A guard *block* is a `guard-*` row whose decision is `deny`: only then was a read or search
//!   actually prevented, so only then were bytes saved. `warn` lets the action through.

use std::collections::BTreeMap;

use serde::Serialize;
use tokred_core::hook_event::{HookEventRecord, decision, reason};

use crate::span::TimeRange;

/// Rough size of a token in bytes, used for every bytes-to-tokens estimate.
pub(crate) const BYTES_PER_TOKEN: u64 = 4;

/// Whether the event is a guard that denied (and so avoided) something.
pub(crate) fn is_guard_block(event: &HookEventRecord) -> bool {
    event.hook.starts_with("guard-") && event.decision == decision::DENY
}

/// How many prompts were classified as each task class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClassCount {
    /// Task class (`architecture`, `implementation`, ...).
    pub class: String,
    /// Prompts classified so.
    pub count: u64,
}

/// What `enforce` saw when a subagent was chosen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Delegations {
    /// The chosen subagent matched the classification.
    pub correct: u64,
    /// It did not, and `enforce` warned or denied.
    pub corrected: u64,
    /// It did not, and the hook let it through anyway (mode `off`).
    pub mismatch_allowed: u64,
    /// `correct + corrected + mismatch_allowed`.
    pub total: u64,
}

/// The "Routing" section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RoutingSummary {
    /// Period covered.
    pub range: TimeRange,
    /// Classifications per task class, most frequent first.
    pub classifications: Vec<ClassCount>,
    /// Total classified prompts.
    pub total_classified: u64,
    /// Subagent delegations.
    pub delegations: Delegations,
}

/// Counts classifications and delegations in `range`.
#[must_use]
pub fn routing_summary(events: &[HookEventRecord], range: TimeRange) -> RoutingSummary {
    let mut classes: BTreeMap<&str, u64> = BTreeMap::new();
    let mut delegations = Delegations::default();
    for event in events.iter().filter(|event| range.contains(event.ts)) {
        match event.hook.as_str() {
            "route" if !matches!(event.decision.as_str(), decision::NONE | "") => {
                *classes.entry(&event.decision).or_default() += 1;
            }
            "enforce" if event.reason == reason::AGENT_MATCHES => delegations.correct += 1,
            "enforce" if event.reason == reason::AGENT_MISMATCH => {
                if matches!(event.decision.as_str(), decision::DENY | decision::WARN) {
                    delegations.corrected += 1;
                } else {
                    delegations.mismatch_allowed += 1;
                }
            }
            _ => {}
        }
    }
    delegations.total = delegations.correct + delegations.corrected + delegations.mismatch_allowed;
    let mut classifications: Vec<ClassCount> = classes
        .into_iter()
        .map(|(class, count)| ClassCount {
            class: class.to_owned(),
            count,
        })
        .collect();
    classifications.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.class.cmp(&b.class)));
    RoutingSummary {
        range,
        total_classified: classifications.iter().map(|c| c.count).sum(),
        classifications,
        delegations,
    }
}

/// Blocks of one guard reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuardReason {
    /// Reason (`reread`, `generated_file`, `large_file`, `broad_search`, ...).
    pub reason: String,
    /// Number of blocks.
    pub blocks: u64,
    /// Bytes of context avoided.
    pub bytes_saved: u64,
    /// Estimated tokens avoided (`bytes_saved / 4`).
    pub tokens_avoided: u64,
}

/// The "Guards" section. Token figures are estimates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuardsSummary {
    /// Period covered.
    pub range: TimeRange,
    /// Always `true`: tokens are estimated from bytes.
    pub estimate: bool,
    /// Blocks per reason, most bytes first.
    pub reasons: Vec<GuardReason>,
    /// Total blocks.
    pub total_blocks: u64,
    /// Total bytes avoided.
    pub total_bytes_saved: u64,
    /// Total estimated tokens avoided.
    pub total_tokens_avoided: u64,
}

/// Groups guard blocks in `range` by reason.
#[must_use]
pub fn guards_summary(events: &[HookEventRecord], range: TimeRange) -> GuardsSummary {
    let mut by_reason: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| range.contains(event.ts) && is_guard_block(event))
    {
        let entry = by_reason.entry(&event.reason).or_default();
        entry.0 += 1;
        entry.1 = entry.1.saturating_add(event.bytes_saved);
    }
    let mut reasons: Vec<GuardReason> = by_reason
        .into_iter()
        .map(|(reason, (blocks, bytes))| GuardReason {
            reason: reason.to_owned(),
            blocks,
            bytes_saved: bytes,
            tokens_avoided: bytes / BYTES_PER_TOKEN,
        })
        .collect();
    reasons.sort_by(|a, b| {
        b.bytes_saved
            .cmp(&a.bytes_saved)
            .then_with(|| a.reason.cmp(&b.reason))
    });
    GuardsSummary {
        range,
        estimate: true,
        total_blocks: reasons.iter().map(|r| r.blocks).sum(),
        total_bytes_saved: reasons.iter().map(|r| r.bytes_saved).sum(),
        total_tokens_avoided: reasons.iter().map(|r| r.tokens_avoided).sum(),
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use tokred_core::Timestamp;
    use tokred_core::hook_event::Outcome;

    use super::*;

    fn event(hook: &str, decision: &str, reason: &str, bytes: u64) -> HookEventRecord {
        HookEventRecord {
            ts: Timestamp(100),
            session_id: "s1".to_owned(),
            hook: hook.to_owned(),
            decision: decision.to_owned(),
            reason: reason.to_owned(),
            bytes_saved: bytes,
            duration_us: 1_000,
            outcome: Outcome::Ok,
        }
    }

    fn everything() -> TimeRange {
        TimeRange::new(Timestamp(0), Timestamp(1_000))
    }

    #[test]
    fn classifications_are_counted_per_class() {
        let events = vec![
            event("route", "implementation", reason::SUGGESTED, 0),
            event("route", "implementation", reason::UNCHANGED, 0),
            event("route", "architecture", reason::SUGGESTED, 0),
            event("route", "search", reason::SUGGESTED, 0),
            event("route", decision::NONE, "", 0),
            event("route", "", "", 0),
            event("guard-read", "deny", reason::REREAD, 10),
        ];
        let summary = routing_summary(&events, everything());
        let got: Vec<_> = summary
            .classifications
            .iter()
            .map(|c| (c.class.as_str(), c.count))
            .collect();
        assert_eq!(
            got,
            [("implementation", 2), ("architecture", 1), ("search", 1)]
        );
        assert_eq!(summary.total_classified, 4);
        assert_eq!(summary.delegations, Delegations::default());
    }

    #[test]
    fn delegations_are_correct_corrected_or_allowed() {
        let events = vec![
            event("enforce", decision::ALLOW, reason::AGENT_MATCHES, 0),
            event("enforce", decision::ALLOW, reason::AGENT_MATCHES, 0),
            event("enforce", decision::DENY, reason::AGENT_MISMATCH, 0),
            event("enforce", decision::WARN, reason::AGENT_MISMATCH, 0),
            event("enforce", decision::ALLOW, reason::AGENT_MISMATCH, 0),
            event("enforce", decision::NONE, "other", 0),
        ];
        let summary = routing_summary(&events, everything());
        assert_eq!(
            summary.delegations,
            Delegations {
                correct: 2,
                corrected: 2,
                mismatch_allowed: 1,
                total: 5
            }
        );
        assert!(summary.classifications.is_empty());
    }

    #[test]
    fn events_outside_the_range_are_ignored() {
        let events = vec![event("route", "review", reason::SUGGESTED, 0)];
        let late = TimeRange::new(Timestamp(101), Timestamp(200));
        assert_eq!(routing_summary(&events, late).total_classified, 0);
        assert_eq!(guards_summary(&events, late).total_blocks, 0);
    }

    #[test]
    fn guard_blocks_are_grouped_by_reason() {
        let events = vec![
            event("guard-read", "deny", reason::REREAD, 4_000),
            event("guard-read", "deny", reason::REREAD, 1_003),
            event("guard-read", "deny", reason::LARGE_FILE, 100_000),
            event("guard-search", "deny", reason::BROAD_SEARCH, 1_003),
            event("guard-search", "warn", reason::BROAD_SEARCH, 9_999),
            event("guard-read", "allow", reason::REREAD, 9_999),
            event("route", "deny", reason::REREAD, 9_999),
        ];
        let summary = guards_summary(&events, everything());
        assert!(summary.estimate);
        let got: Vec<_> = summary
            .reasons
            .iter()
            .map(|r| (r.reason.as_str(), r.blocks, r.bytes_saved, r.tokens_avoided))
            .collect();
        assert_eq!(
            got,
            [
                ("large_file", 1, 100_000, 25_000),
                ("reread", 2, 5_003, 1_250),
                ("broad_search", 1, 1_003, 250),
            ]
        );
        assert_eq!(summary.total_blocks, 4);
        assert_eq!(summary.total_bytes_saved, 106_006);
        assert_eq!(summary.total_tokens_avoided, 26_500);
    }

    #[test]
    fn ties_in_bytes_sort_by_reason() {
        let events = vec![
            event("guard-read", "deny", "b", 8),
            event("guard-read", "deny", "a", 8),
        ];
        let summary = guards_summary(&events, everything());
        assert_eq!(summary.reasons[0].reason, "a");
    }

    #[test]
    fn empty_input_gives_empty_summaries() {
        let guards = guards_summary(&[], everything());
        assert_eq!((guards.total_blocks, guards.reasons.len()), (0, 0));
        assert_eq!(routing_summary(&[], everything()).delegations.total, 0);
    }
}
