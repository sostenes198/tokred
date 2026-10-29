//! What only the proxy knows (spec 6.4 and 7.2): where the numbers came from, the model each
//! request asked for against the one that answered, what routing saved, what the proxy itself
//! cost, why the prompt cache missed, and the suggestion waiting to be read (pure).
//!
//! Everything here works on plain slices and is limited to the range, like the rest of `stats`.
//! [`proxy_stats`] returns `None` when the proxy recorded nothing in the range, so a report
//! without the proxy stays exactly what it was.

use std::collections::BTreeMap;

use serde::Serialize;
use tokred_core::Timestamp;
use tokred_core::pricing::{PriceTable, Usage, cost as usage_cost};
use tokred_core::prompts::SuggestionRecord;
use tokred_core::proxy::ProxyRequestRecord;
use tokred_core::routing::RoutingDecision;
use tokred_core::usage::UsageRecord;

use crate::latency::{ms, percentile};
use crate::span::TimeRange;
use crate::util::{len_u64, ratio, round6};

/// What the proxy may add to a request, p95, in milliseconds (spec 6.7).
pub const OVERHEAD_BUDGET_MS: f64 = 10.0;

/// The request the proxy answered with a 2xx status and that has tokens.
fn is_message(request: &ProxyRequestRecord) -> bool {
    (200..300).contains(&request.status) && request.usage.total() > 0
}

/// Everything [`proxy_stats`] needs.
#[derive(Debug, Clone, Copy)]
pub struct ProxyInput<'a> {
    /// The merged usage rows (see [`crate::merge_usage`]).
    pub usage: &'a [UsageRecord],
    /// The proxy requests (any period; the range is applied here).
    pub requests: &'a [ProxyRequestRecord],
    /// The routing decisions (any period).
    pub decisions: &'a [RoutingDecision],
    /// The suggestions (any period).
    pub suggestions: &'a [SuggestionRecord],
    /// Prices.
    pub prices: &'a PriceTable,
    /// The period reported.
    pub range: TimeRange,
}

/// Where the messages of the period were measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Coverage {
    /// Messages the proxy measured (exact).
    pub measured: u64,
    /// Messages known only from the transcripts (made while the proxy was off).
    pub transcript_only: u64,
}

/// One pair of requested and effective model.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelRoute {
    /// The model Claude Code asked for.
    pub requested: String,
    /// The model that answered.
    pub effective: String,
    /// The two differ: the proxy rewrote the request.
    pub rewritten: bool,
    /// Messages.
    pub requests: u64,
    /// Tokens.
    pub tokens: u64,
    /// What they cost on the model that answered, in USD.
    pub cost_usd: f64,
}

/// What routing decided in the period and what it was worth.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct RoutingOutcomes {
    /// Decisions taken.
    pub decisions: u64,
    /// The model was changed (`enforce`).
    pub applied: u64,
    /// The model would have been changed (`observe`).
    pub observed: u64,
    /// A cheaper model was not used (kept as asked).
    pub kept: u64,
    /// A rewrite was rejected or stopped and the original went through.
    pub fallback: u64,
    /// Estimated saving of the applied decisions, USD.
    pub applied_saving_usd: f64,
    /// Estimated saving the observed decisions would have made, USD.
    pub observed_saving_usd: f64,
}

/// The time the proxy added to the requests it forwarded.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Overhead {
    /// Requests measured.
    pub requests: u64,
    /// Median, ms.
    pub p50_ms: f64,
    /// 95th percentile, ms.
    pub p95_ms: f64,
    /// 99th percentile, ms.
    pub p99_ms: f64,
    /// Slowest, ms.
    pub max_ms: f64,
    /// The budget for the p95, ms.
    pub budget_ms: f64,
    /// The p95 is over the budget (exactly the budget is within it).
    pub over_budget: bool,
}

/// One likely cause of main-loop prompt-cache misses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MissCause {
    /// The cause (`ttl_expired`, `tools_changed`, ...).
    pub cause: String,
    /// Requests that missed for it.
    pub requests: u64,
    /// Tokens those requests wrote to the cache again.
    pub cache_write_tokens: u64,
}

/// Prompt-cache misses of the main loop, by cause.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CacheMisses {
    /// Main-loop messages in the period.
    pub main_requests: u64,
    /// Of those, the ones with a likely miss cause.
    pub misses: u64,
    /// `misses / main_requests`.
    pub rate: Option<f64>,
    /// Misses per cause, most frequent first.
    pub causes: Vec<MissCause>,
}

/// A suggestion not shown anywhere yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingSuggestion {
    /// Its id.
    pub id: String,
    /// When it was made.
    pub ts: i64,
    /// The session it is about.
    pub session_id: String,
    /// The one-line message.
    pub message: String,
}

/// The proxy's view of the period.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProxyStats {
    /// Where the messages were measured.
    pub coverage: Coverage,
    /// Requests the proxy handled, whatever the answer.
    pub requests: u64,
    /// Requests the upstream did not answer with a 2xx status.
    pub failed: u64,
    /// Requested against effective model, most requests first.
    pub models: Vec<ModelRoute>,
    /// What routing decided (`None` without decisions).
    pub routing: Option<RoutingOutcomes>,
    /// The proxy's own overhead (`None` without requests).
    pub overhead: Option<Overhead>,
    /// Cache misses of the main loop (`None` without misses).
    pub cache_misses: Option<CacheMisses>,
    /// The latest suggestion not shown yet.
    pub suggestion: Option<PendingSuggestion>,
}

fn coverage(usage: &[UsageRecord], range: TimeRange) -> Coverage {
    let in_range = usage.iter().filter(|row| range.contains(row.ts));
    let (measured, transcript_only) = in_range.fold((0, 0), |(measured, only), row| {
        if row.origin.is_some() {
            (measured + 1, only)
        } else {
            (measured, only + 1)
        }
    });
    Coverage {
        measured,
        transcript_only,
    }
}

fn models(requests: &[&ProxyRequestRecord], prices: &PriceTable) -> Vec<ModelRoute> {
    let mut pairs: BTreeMap<(&str, &str), (u64, Usage)> = BTreeMap::new();
    for request in requests.iter().filter(|request| is_message(request)) {
        let entry = pairs
            .entry((&request.model_requested, &request.model_effective))
            .or_default();
        entry.0 += 1;
        entry.1.add(&request.usage);
    }
    let mut routes: Vec<ModelRoute> = pairs
        .into_iter()
        .map(|((requested, effective), (count, usage))| ModelRoute {
            requested: requested.to_owned(),
            effective: effective.to_owned(),
            rewritten: requested != effective,
            requests: count,
            tokens: usage.total(),
            cost_usd: round6(
                prices
                    .resolve(effective)
                    .map_or(0.0, |resolved| usage_cost(&usage, &resolved.price)),
            ),
        })
        .collect();
    routes.sort_by_key(|route| std::cmp::Reverse(route.requests));
    routes
}

fn routing(decisions: &[&RoutingDecision]) -> Option<RoutingOutcomes> {
    if decisions.is_empty() {
        return None;
    }
    let count = |outcome: &str| len_u64(decisions.iter().filter(|d| d.outcome == outcome).count());
    // `+ 0.0` turns the `-0.0` of an empty float sum into `0.0`.
    let saving = |outcome: &str| {
        round6(
            decisions
                .iter()
                .filter(|d| d.outcome == outcome)
                .map(|d| d.est_cost_from - d.est_cost_to)
                .sum::<f64>()
                + 0.0,
        )
    };
    Some(RoutingOutcomes {
        decisions: len_u64(decisions.len()),
        applied: count("applied"),
        observed: count("observed"),
        kept: count("kept"),
        fallback: count("fallback"),
        applied_saving_usd: saving("applied"),
        observed_saving_usd: saving("observed"),
    })
}

fn overhead(requests: &[&ProxyRequestRecord]) -> Option<Overhead> {
    if requests.is_empty() {
        return None;
    }
    let mut samples: Vec<u64> = requests.iter().map(|r| r.proxy_overhead_us).collect();
    samples.sort_unstable();
    let p95 = ms(percentile(&samples, 95));
    Some(Overhead {
        requests: len_u64(samples.len()),
        p50_ms: ms(percentile(&samples, 50)),
        p95_ms: p95,
        p99_ms: ms(percentile(&samples, 99)),
        max_ms: ms(percentile(&samples, 100)),
        budget_ms: OVERHEAD_BUDGET_MS,
        over_budget: p95 > OVERHEAD_BUDGET_MS,
    })
}

fn cache_misses(requests: &[&ProxyRequestRecord]) -> Option<CacheMisses> {
    let main: Vec<&&ProxyRequestRecord> = requests
        .iter()
        .filter(|request| is_message(request) && request.origin == "main")
        .collect();
    let mut causes: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
    for request in &main {
        if let Some(cause) = request.miss_cause.as_deref() {
            let entry = causes.entry(cause).or_default();
            entry.0 += 1;
            entry.1 += request.usage.cache_write;
        }
    }
    let misses: u64 = causes.values().map(|(count, _)| count).sum();
    if misses == 0 {
        return None;
    }
    let main_requests = len_u64(main.len());
    let mut causes: Vec<MissCause> = causes
        .into_iter()
        .map(|(cause, (requests, cache_write_tokens))| MissCause {
            cause: cause.to_owned(),
            requests,
            cache_write_tokens,
        })
        .collect();
    causes.sort_by_key(|cause| std::cmp::Reverse(cause.requests));
    Some(CacheMisses {
        main_requests,
        misses,
        rate: ratio(misses, main_requests),
        causes,
    })
}

fn pending(suggestions: &[&SuggestionRecord]) -> Option<PendingSuggestion> {
    suggestions
        .iter()
        .filter(|suggestion| !suggestion.shown)
        .max_by(|a, b| (a.ts, &a.id).cmp(&(b.ts, &b.id)))
        .map(|suggestion| PendingSuggestion {
            id: suggestion.id.clone(),
            ts: suggestion.ts.0,
            session_id: suggestion.session_id.clone(),
            message: suggestion.message.clone(),
        })
}

/// The proxy's view of the range; `None` when it recorded nothing in it and nothing waits to be
/// read.
#[must_use]
pub fn proxy_stats(input: &ProxyInput<'_>) -> Option<ProxyStats> {
    let within = |ts: Timestamp| input.range.contains(ts);
    let requests: Vec<&ProxyRequestRecord> = input
        .requests
        .iter()
        .filter(|request| within(request.ts))
        .collect();
    let decisions: Vec<&RoutingDecision> = input
        .decisions
        .iter()
        .filter(|decision| within(decision.ts))
        .collect();
    let suggestions: Vec<&SuggestionRecord> = input
        .suggestions
        .iter()
        .filter(|suggestion| within(suggestion.ts))
        .collect();
    let suggestion = pending(&suggestions);
    if requests.is_empty() && decisions.is_empty() && suggestion.is_none() {
        return None;
    }
    Some(ProxyStats {
        coverage: coverage(input.usage, input.range),
        requests: len_u64(requests.len()),
        failed: len_u64(
            requests
                .iter()
                .filter(|request| !(200..300).contains(&request.status))
                .count(),
        ),
        models: models(&requests, input.prices),
        routing: routing(&decisions),
        overhead: overhead(&requests),
        cache_misses: cache_misses(&requests),
        suggestion,
    })
}

#[cfg(test)]
mod tests {
    use tokred_core::pricing::{ModelPrice, PriceEntry};

    use super::*;

    const HAIKU: &str = "claude-haiku-4-5";
    const OPUS: &str = "claude-opus-4-1-20250805";

    fn prices() -> PriceTable {
        let entry = |model: &str, input: f64| PriceEntry {
            model: model.to_owned(),
            price: ModelPrice {
                input,
                output: input * 5.0,
                cache_write: input * 1.25,
                cache_write_1h: input * 2.0,
                cache_read: input / 10.0,
            },
            source: "test".to_owned(),
            updated_at: Timestamp(0),
        };
        PriceTable::new(vec![entry(HAIKU, 1.0), entry(OPUS, 15.0)])
    }

    fn range() -> TimeRange {
        TimeRange::new(Timestamp(100), Timestamp(200))
    }

    fn request(id: &str, ts: i64) -> ProxyRequestRecord {
        ProxyRequestRecord {
            id: id.to_owned(),
            ts: Timestamp(ts),
            origin: "main".to_owned(),
            model_requested: OPUS.to_owned(),
            model_effective: OPUS.to_owned(),
            usage: Usage {
                input: 1_000_000,
                output: 0,
                cache_write: 0,
                cache_write_1h: 0,
                cache_read: 0,
            },
            status: 200,
            ..ProxyRequestRecord::default()
        }
    }

    fn decision(id: &str, ts: i64, outcome: &str, from: f64, to: f64) -> RoutingDecision {
        RoutingDecision {
            id: id.to_owned(),
            ts: Timestamp(ts),
            outcome: outcome.to_owned(),
            est_cost_from: from,
            est_cost_to: to,
            ..RoutingDecision::default()
        }
    }

    fn suggestion(id: &str, ts: i64, shown: bool) -> SuggestionRecord {
        SuggestionRecord {
            id: id.to_owned(),
            ts: Timestamp(ts),
            session_id: "s1".to_owned(),
            message: format!("use haiku ({id})"),
            shown,
            ..SuggestionRecord::default()
        }
    }

    fn stats_of(
        requests: &[ProxyRequestRecord],
        decisions: &[RoutingDecision],
        suggestions: &[SuggestionRecord],
        usage: &[UsageRecord],
    ) -> Option<ProxyStats> {
        proxy_stats(&ProxyInput {
            usage,
            requests,
            decisions,
            suggestions,
            prices: &prices(),
            range: range(),
        })
    }

    #[test]
    fn nothing_in_the_range_is_nothing() {
        assert_eq!(stats_of(&[], &[], &[], &[]), None);
        // Records of other periods do not count, and a shown suggestion is not pending.
        let outside = [request("p", 50), request("q", 200)];
        let old_decision = [decision("d", 99, "applied", 1.0, 0.5)];
        let shown = [suggestion("s", 150, true)];
        assert_eq!(stats_of(&outside, &old_decision, &shown, &[]), None);
    }

    #[test]
    fn a_pending_suggestion_alone_is_enough_and_the_latest_wins() {
        let suggestions = [
            suggestion("a", 120, false),
            suggestion("b", 150, false),
            suggestion("c", 160, true),
            suggestion("d", 150, false),
            suggestion("old", 10, false),
        ];
        let stats = stats_of(&[], &[], &suggestions, &[]).unwrap();
        let pending = stats.suggestion.unwrap();
        assert_eq!(pending.id, "d", "newest first, then the larger id");
        assert_eq!(pending.message, "use haiku (d)");
        assert_eq!(pending.session_id, "s1");
        assert_eq!(pending.ts, 150);
        assert_eq!(stats.requests, 0);
        assert!(stats.overhead.is_none() && stats.routing.is_none() && stats.models.is_empty());
    }

    #[test]
    fn coverage_counts_measured_and_transcript_only_messages_of_the_range() {
        let row = |ts: i64, origin: Option<&str>| UsageRecord {
            message_key: format!("k{ts}"),
            session_id: "s".to_owned(),
            project: "p".to_owned(),
            ts: Timestamp(ts),
            model: OPUS.to_owned(),
            usage: Usage::default(),
            is_subagent: false,
            origin: origin.map(str::to_owned),
            agent_type: None,
        };
        let usage = [
            row(120, Some("main")),
            row(130, None),
            row(140, Some("subagent")),
            row(500, None),
        ];
        let stats = stats_of(&[request("p", 150)], &[], &[], &usage).unwrap();
        assert_eq!(
            stats.coverage,
            Coverage {
                measured: 2,
                transcript_only: 1
            }
        );
    }

    #[test]
    fn the_models_show_what_was_asked_and_what_answered() {
        let mut rewritten = request("p2", 130);
        rewritten.model_effective = HAIKU.to_owned();
        let mut rewritten_again = request("p3", 131);
        rewritten_again.model_effective = HAIKU.to_owned();
        let mut failed = request("p4", 132);
        failed.status = 529;
        failed.usage = Usage::default();
        let requests = [request("p1", 120), rewritten, rewritten_again, failed];
        let stats = stats_of(&requests, &[], &[], &[]).unwrap();

        assert_eq!(stats.requests, 4);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.models.len(), 2, "the failed request is not a message");
        let first = &stats.models[0];
        assert_eq!(
            (first.requested.as_str(), first.effective.as_str()),
            (OPUS, HAIKU)
        );
        assert!(first.rewritten);
        assert_eq!(first.requests, 2);
        assert_eq!(first.tokens, 2_000_000);
        assert!(
            (first.cost_usd - 2.0).abs() < 1e-9,
            "2M input tokens at haiku's $1/M"
        );
        let second = &stats.models[1];
        assert!(!second.rewritten);
        assert!((second.cost_usd - 15.0).abs() < 1e-9);

        let mut unknown = request("p5", 140);
        unknown.model_effective = "mystery".to_owned();
        let stats = stats_of(&[unknown], &[], &[], &[]).unwrap();
        assert!(
            stats.models[0].cost_usd.abs() < f64::EPSILON,
            "an unpriced model costs nothing"
        );
    }

    #[test]
    fn routing_separates_what_was_applied_from_what_was_only_observed() {
        let decisions = [
            decision("d1", 110, "applied", 0.10, 0.02),
            decision("d2", 120, "applied", 0.05, 0.01),
            decision("d3", 130, "observed", 0.30, 0.10),
            decision("d4", 140, "kept", 0.1, 0.1),
            decision("d5", 150, "fallback", 0.1, 0.1),
            decision("outside", 400, "applied", 9.0, 0.0),
        ];
        let routing = stats_of(&[], &decisions, &[], &[])
            .unwrap()
            .routing
            .unwrap();
        assert_eq!(routing.decisions, 5);
        assert_eq!(
            (
                routing.applied,
                routing.observed,
                routing.kept,
                routing.fallback
            ),
            (2, 1, 1, 1)
        );
        assert!((routing.applied_saving_usd - 0.12).abs() < 1e-9);
        assert!((routing.observed_saving_usd - 0.20).abs() < 1e-9);

        let only_kept = [decision("k", 110, "kept", 0.1, 0.1)];
        let routing = stats_of(&[], &only_kept, &[], &[])
            .unwrap()
            .routing
            .unwrap();
        assert_eq!(
            routing.applied_saving_usd.to_bits(),
            0.0_f64.to_bits(),
            "never -0.0"
        );
    }

    #[test]
    fn the_overhead_is_measured_against_its_budget() {
        let with_overhead = |us: &[u64]| -> Overhead {
            let requests: Vec<_> = us
                .iter()
                .zip(0..)
                .map(|(&us, n)| {
                    let mut r = request(&format!("p{n}"), 120);
                    r.proxy_overhead_us = us;
                    r
                })
                .collect();
            stats_of(&requests, &[], &[], &[])
                .unwrap()
                .overhead
                .unwrap()
        };

        let quick = with_overhead(&[400, 300, 500, 450, 350]);
        assert_eq!(quick.requests, 5);
        assert!((quick.p50_ms - 0.4).abs() < 1e-9);
        assert!((quick.p95_ms - 0.5).abs() < 1e-9);
        assert!((quick.max_ms - 0.5).abs() < 1e-9);
        assert!((quick.budget_ms - 10.0).abs() < f64::EPSILON);
        assert!(!quick.over_budget);

        let exactly = with_overhead(&[10_000]);
        assert!(!exactly.over_budget, "exactly the budget is within it");
        let slow = with_overhead(&[10_001]);
        assert!(slow.over_budget);
        let mixed: Vec<u64> = (1..=100)
            .map(|n| if n > 95 { 50_000 } else { 100 })
            .collect();
        let outliers = with_overhead(&mixed);
        assert!(!outliers.over_budget, "p95 ignores the slowest 5%");
        assert!((outliers.p99_ms - 50.0).abs() < 1e-9);
        let more: Vec<u64> = (1..=100)
            .map(|n| if n > 94 { 50_000 } else { 100 })
            .collect();
        assert!(with_overhead(&more).over_budget);
    }

    #[test]
    fn cache_misses_are_counted_per_cause_for_the_main_loop() {
        let miss = |id: &str, cause: Option<&str>, origin: &str, written: u64| {
            let mut r = request(id, 120);
            r.miss_cause = cause.map(str::to_owned);
            r.origin = origin.to_owned();
            r.usage.cache_write = written;
            r
        };
        let requests = [
            miss("a", Some("ttl_expired"), "main", 1_000),
            miss("b", Some("ttl_expired"), "main", 2_000),
            miss("c", Some("tools_changed"), "main", 500),
            miss("d", None, "main", 0),
            miss("e", Some("ttl_expired"), "subagent", 9_999),
        ];
        let misses = stats_of(&requests, &[], &[], &[])
            .unwrap()
            .cache_misses
            .unwrap();
        assert_eq!((misses.main_requests, misses.misses), (4, 3));
        assert_eq!(misses.rate, Some(0.75));
        assert_eq!(misses.causes.len(), 2);
        assert_eq!(misses.causes[0].cause, "ttl_expired");
        assert_eq!(misses.causes[0].requests, 2);
        assert_eq!(misses.causes[0].cache_write_tokens, 3_000);
        assert_eq!(misses.causes[1].cause, "tools_changed");

        let none = [miss("a", None, "main", 0)];
        assert_eq!(stats_of(&none, &[], &[], &[]).unwrap().cache_misses, None);
    }
}
