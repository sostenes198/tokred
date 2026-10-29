//! `tokred stats latency`: how long the hooks delayed Claude Code (pure).
//!
//! Each run's cost is its recorded `duration_us` plus the process-spawn overhead measured by
//! `doctor --bench`, which is not visible from inside the binary and is therefore an estimate.
//! Percentiles use the nearest-rank method on integer microseconds: the p-th percentile of `n`
//! sorted samples is the sample at rank `ceil(p * n / 100)` (1-based), so a single sample is
//! every percentile and ties return the tied value.

use std::collections::BTreeMap;

use serde::Serialize;
use tokred_core::hook_event::{HookEventRecord, Outcome};

use crate::span::TimeRange;
use crate::stats::Stats;
use crate::util::{day_string, len_u64, round6, to_f64};

/// A hook whose p95 passes this many milliseconds raises an alert.
pub const BUDGET_MS: f64 = 50.0;
const BUDGET_US: u64 = 50_000;

/// Latency of one hook.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HookLatency {
    /// Hook name.
    pub hook: String,
    /// Executions.
    pub runs: u64,
    /// Median, ms (spawn overhead included).
    pub p50_ms: f64,
    /// 95th percentile, ms.
    pub p95_ms: f64,
    /// 99th percentile, ms.
    pub p99_ms: f64,
    /// Slowest run, ms.
    pub max_ms: f64,
    /// Sum over all runs, ms.
    pub total_ms: f64,
}

/// Time added in one session or one day.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bucket {
    /// Session id or UTC day.
    pub key: String,
    /// Executions.
    pub runs: u64,
    /// Total ms added.
    pub total_ms: f64,
}

/// Executions per outcome.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct OutcomeCounts {
    /// Finished normally.
    pub ok: u64,
    /// Failed with an error.
    pub error: u64,
    /// Panicked.
    pub panic: u64,
    /// Stopped by the time guard.
    pub timeout_guard: u64,
}

/// A hook that went over the latency budget.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BudgetAlert {
    /// Hook name.
    pub hook: String,
    /// Its p95 in ms.
    pub p95_ms: f64,
}

/// Result of [`latency_stats`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LatencyStats {
    /// Period reported.
    pub range: TimeRange,
    /// Always `true`: the spawn overhead is an estimate from `doctor --bench`.
    pub estimate: bool,
    /// Spawn overhead added to every run, in microseconds.
    pub spawn_overhead_us: u64,
    /// Executions in the range.
    pub runs: u64,
    /// Total ms added by all hooks.
    pub total_ms: f64,
    /// Per hook, by name.
    pub hooks: Vec<HookLatency>,
    /// Per session, by id.
    pub per_session: Vec<Bucket>,
    /// Per UTC day, oldest first.
    pub per_day: Vec<Bucket>,
    /// User prompts (`route` / `UserPromptSubmit` executions).
    pub prompts: u64,
    /// Average ms added per user prompt (`total_ms / prompts`), `None` without prompts.
    pub avg_ms_per_prompt: Option<f64>,
    /// Executions per outcome.
    pub outcomes: OutcomeCounts,
    /// The budget in ms (p95).
    pub budget_ms: f64,
    /// Hooks over the budget.
    pub budget_alerts: Vec<BudgetAlert>,
}

/// Microseconds as milliseconds, rounded to microsecond precision.
pub(crate) fn ms(us: u64) -> f64 {
    round6(to_f64(us) / 1_000.0)
}

/// Nearest-rank percentile `p` (0..=100) of ascending `sorted` samples; 0 when empty.
pub(crate) fn percentile(sorted: &[u64], p: usize) -> u64 {
    let rank = (p * sorted.len()).div_ceil(100).max(1);
    sorted.get(rank - 1).copied().unwrap_or(0)
}

fn buckets(totals: BTreeMap<String, (u64, u64)>) -> Vec<Bucket> {
    totals
        .into_iter()
        .map(|(key, (runs, us))| Bucket {
            key,
            runs,
            total_ms: ms(us),
        })
        .collect()
}

/// Summarizes the hook executions in `range`. `spawn_overhead_us` is added to each run.
#[must_use]
pub fn latency_stats(
    events: &[HookEventRecord],
    spawn_overhead_us: u64,
    range: TimeRange,
) -> LatencyStats {
    let mut samples: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
    let mut sessions: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut days: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut outcomes = OutcomeCounts::default();
    let mut prompts = 0;
    let mut total_us: u64 = 0;
    let mut runs = 0;
    for event in events.iter().filter(|event| range.contains(event.ts)) {
        let us = event.duration_us.saturating_add(spawn_overhead_us);
        samples.entry(&event.hook).or_default().push(us);
        for entry in [
            sessions.entry(event.session_id.clone()).or_default(),
            days.entry(day_string(event.ts.0)).or_default(),
        ] {
            entry.0 += 1;
            entry.1 = entry.1.saturating_add(us);
        }
        match event.outcome {
            Outcome::Ok => outcomes.ok += 1,
            Outcome::Error => outcomes.error += 1,
            Outcome::Panic => outcomes.panic += 1,
            Outcome::TimeoutGuard => outcomes.timeout_guard += 1,
        }
        prompts += u64::from(matches!(event.hook.as_str(), "route" | "UserPromptSubmit"));
        total_us = total_us.saturating_add(us);
        runs += 1;
    }

    let mut budget_alerts = Vec::new();
    let hooks = samples
        .into_iter()
        .map(|(hook, mut values)| {
            values.sort_unstable();
            let p95 = percentile(&values, 95);
            if p95 > BUDGET_US {
                budget_alerts.push(BudgetAlert {
                    hook: hook.to_owned(),
                    p95_ms: ms(p95),
                });
            }
            HookLatency {
                hook: hook.to_owned(),
                runs: len_u64(values.len()),
                p50_ms: ms(percentile(&values, 50)),
                p95_ms: ms(p95),
                p99_ms: ms(percentile(&values, 99)),
                max_ms: ms(percentile(&values, 100)),
                total_ms: ms(values.iter().sum()),
            }
        })
        .collect();

    LatencyStats {
        range,
        estimate: true,
        spawn_overhead_us,
        runs,
        total_ms: ms(total_us),
        hooks,
        per_session: buckets(sessions),
        per_day: buckets(days),
        prompts,
        avg_ms_per_prompt: (prompts != 0).then(|| round6(ms(total_us) / to_f64(prompts))),
        outcomes,
        budget_ms: BUDGET_MS,
        budget_alerts,
    }
}

/// Latency against what tokred saved in the same period.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Tradeoff {
    /// Always `true`: the savings side includes estimates.
    pub estimate: bool,
    /// Total ms added by the hooks.
    pub ms_added: f64,
    /// Estimated tokens the guards avoided.
    pub tokens_avoided: u64,
    /// Routing plus guard saving in USD (`None` when the routing saving is unknown).
    pub saving_usd: Option<f64>,
}

/// Puts the latency added next to the tokens and cost saved. Use the same range for both inputs.
#[must_use]
pub fn tradeoff(latency: &LatencyStats, stats: &Stats) -> Tradeoff {
    Tradeoff {
        estimate: true,
        ms_added: latency.total_ms,
        tokens_avoided: stats.savings.guard.tokens_avoided,
        saving_usd: stats.savings.total_saving_usd,
    }
}

#[cfg(test)]
mod tests {
    use tokred_core::Timestamp;
    use tokred_core::pricing::PriceTable;

    use super::*;
    use crate::stats::{GroupBy, StatsInput, compute_stats};

    /// 2026-03-01T00:00:00Z.
    const T0: i64 = 1_772_323_200;

    fn event(
        hook: &str,
        session: &str,
        ts: i64,
        duration_us: u64,
        outcome: Outcome,
    ) -> HookEventRecord {
        HookEventRecord {
            ts: Timestamp(ts),
            session_id: session.to_owned(),
            hook: hook.to_owned(),
            decision: "allow".to_owned(),
            reason: String::new(),
            bytes_saved: 0,
            duration_us,
            outcome,
        }
    }

    fn whole() -> TimeRange {
        TimeRange::new(Timestamp(T0), Timestamp(T0 + 10 * 86_400))
    }

    #[test]
    fn percentiles_use_nearest_rank() {
        assert_eq!(percentile(&[], 50), 0);
        assert_eq!(percentile(&[7], 50), 7);
        assert_eq!(percentile(&[7], 99), 7);
        assert_eq!(percentile(&[7], 100), 7);
        let ten: Vec<u64> = (1..=10).collect();
        assert_eq!(percentile(&ten, 50), 5);
        assert_eq!(percentile(&ten, 95), 10);
        assert_eq!(percentile(&ten, 99), 10);
        let hundred: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile(&hundred, 50), 50);
        assert_eq!(percentile(&hundred, 95), 95);
        assert_eq!(percentile(&hundred, 99), 99);
        assert_eq!(
            percentile(&[4, 4, 4, 9], 50),
            4,
            "ties return the tied value"
        );
        assert_eq!(percentile(&[4, 4, 4, 9], 100), 9);
    }

    #[test]
    fn per_hook_statistics_include_the_spawn_overhead() {
        let mut events = Vec::new();
        // guard-read: 1..=10 ms of internal time, plus 2 ms of spawn overhead.
        for i in 1_u64..=10 {
            events.push(event(
                "guard-read",
                "s1",
                T0 + i64::try_from(i).unwrap(),
                i * 1_000,
                Outcome::Ok,
            ));
        }
        events.push(event("route", "s1", T0 + 20, 500, Outcome::Ok));
        let stats = latency_stats(&events, 2_000, whole());

        assert!(stats.estimate);
        assert_eq!(stats.spawn_overhead_us, 2_000);
        assert_eq!(stats.runs, 11);
        let guard = &stats.hooks[0];
        assert_eq!(guard.hook, "guard-read");
        assert_eq!(guard.runs, 10);
        assert_eq!(guard.p50_ms, 7.0);
        assert_eq!(guard.p95_ms, 12.0);
        assert_eq!(guard.p99_ms, 12.0);
        assert_eq!(guard.max_ms, 12.0);
        assert_eq!(guard.total_ms, 75.0);
        let route = &stats.hooks[1];
        assert_eq!((route.hook.as_str(), route.runs), ("route", 1));
        assert_eq!(
            (route.p50_ms, route.p95_ms, route.p99_ms, route.max_ms),
            (2.5, 2.5, 2.5, 2.5)
        );
        assert_eq!(stats.total_ms, 77.5);
        assert!(stats.budget_alerts.is_empty());
        assert_eq!(stats.budget_ms, 50.0);
    }

    #[test]
    fn totals_per_session_and_day_and_per_prompt() {
        let events = vec![
            event("route", "s1", T0 + 1, 1_000, Outcome::Ok),
            event("guard-read", "s1", T0 + 2, 3_000, Outcome::Ok),
            event("UserPromptSubmit", "s2", T0 + 86_400, 2_000, Outcome::Ok),
            event("session-start", "s2", T0 + 86_401, 4_000, Outcome::Ok),
            event("route", "s2", T0 + 20 * 86_400, 9_000, Outcome::Ok),
        ];
        let stats = latency_stats(&events, 0, whole());
        let sessions: Vec<_> = stats
            .per_session
            .iter()
            .map(|b| (b.key.as_str(), b.runs, b.total_ms))
            .collect();
        assert_eq!(sessions, [("s1", 2, 4.0), ("s2", 2, 6.0)]);
        let days: Vec<_> = stats
            .per_day
            .iter()
            .map(|b| (b.key.as_str(), b.runs, b.total_ms))
            .collect();
        assert_eq!(days, [("2026-03-01", 2, 4.0), ("2026-03-02", 2, 6.0)]);
        assert_eq!(stats.prompts, 2);
        assert_eq!(stats.avg_ms_per_prompt, Some(5.0));
        assert_eq!(stats.runs, 4, "the event after the range is ignored");
    }

    #[test]
    fn outcomes_are_counted_and_slow_hooks_raise_an_alert() {
        let events = vec![
            event("guard-read", "s1", T0, 60_000, Outcome::Ok),
            event("guard-read", "s1", T0, 80_000, Outcome::Error),
            event("route", "s1", T0, 49_000, Outcome::Panic),
            event("route", "s1", T0, 50_000, Outcome::TimeoutGuard),
        ];
        let stats = latency_stats(&events, 0, whole());
        assert_eq!(
            stats.outcomes,
            OutcomeCounts {
                ok: 1,
                error: 1,
                panic: 1,
                timeout_guard: 1
            }
        );
        assert_eq!(
            stats.budget_alerts,
            [BudgetAlert {
                hook: "guard-read".to_owned(),
                p95_ms: 80.0
            }],
            "exactly 50 ms is within budget"
        );
        // The spawn overhead can push a hook over the budget.
        let overhead = latency_stats(&events, 2_000, whole());
        assert_eq!(overhead.budget_alerts.len(), 2);
    }

    #[test]
    fn no_events_means_no_division_by_zero() {
        let stats = latency_stats(&[], 1_500, whole());
        assert_eq!(stats.runs, 0);
        assert_eq!(stats.total_ms, 0.0);
        assert!(stats.hooks.is_empty() && stats.per_day.is_empty());
        assert_eq!(stats.prompts, 0);
        assert_eq!(stats.avg_ms_per_prompt, None);
        assert_eq!(stats.outcomes, OutcomeCounts::default());
    }

    #[test]
    fn tradeoff_sets_latency_against_savings() {
        let latency = latency_stats(
            &[event("guard-read", "s1", T0, 5_000, Outcome::Ok)],
            0,
            whole(),
        );
        let stats = compute_stats(&StatsInput {
            usage: &[],
            prices: &PriceTable::default(),
            events: &[],
            range: whole(),
            group_by: GroupBy::Day,
            now: Timestamp(T0),
            first_configure: None,
        });
        let got = tradeoff(&latency, &stats);
        assert_eq!(
            got,
            Tradeoff {
                estimate: true,
                ms_added: 5.0,
                tokens_avoided: 0,
                saving_usd: None
            }
        );
    }
}
