//! The proxy and the transcripts of one conversation, merged, then `stats` and the proxy
//! sections against a manual sum.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokred_core::Timestamp;
use tokred_core::pricing::{ModelPrice, PriceEntry, PriceTable, Usage};
use tokred_core::prompts::SuggestionRecord;
use tokred_core::proxy::ProxyRequestRecord;
use tokred_core::routing::RoutingDecision;
use tokred_core::usage::UsageRecord;
use tokred_metrics::proxy::{ProxyInput, proxy_stats};
use tokred_metrics::{GroupBy, StatsInput, TimeRange, compute_stats, merge_usage};

/// 2026-03-01T00:00:00Z.
const T0: i64 = 1_772_323_200;
const OPUS: &str = "claude-opus-4-1-20250805";
const SONNET: &str = "claude-sonnet-4-20250514";
const HAIKU: &str = "claude-haiku-4-5";

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
    PriceTable::new(vec![
        entry(OPUS, 15.0),
        entry(SONNET, 3.0),
        entry(HAIKU, 1.0),
    ])
}

fn tokens(input: u64) -> Usage {
    Usage {
        input,
        ..Usage::default()
    }
}

fn transcript(key: &str, ts: i64, model: &str, usage: Usage, subagent: bool) -> UsageRecord {
    UsageRecord {
        message_key: key.to_owned(),
        session_id: "s1".to_owned(),
        project: "-Users-me-app".to_owned(),
        ts: Timestamp(T0 + ts),
        model: model.to_owned(),
        usage,
        is_subagent: subagent,
        origin: None,
        agent_type: None,
    }
}

/// What differs between the requests of the fixture.
struct Spec<'a> {
    id: &'a str,
    key: Option<&'a str>,
    ts: i64,
    origin: &'a str,
    asked: &'a str,
    answered: &'a str,
    input: u64,
    miss: Option<&'a str>,
    overhead_us: u64,
}

fn proxied(spec: &Spec<'_>) -> ProxyRequestRecord {
    ProxyRequestRecord {
        id: spec.id.to_owned(),
        ts: Timestamp(T0 + spec.ts),
        session_id: "s1".to_owned(),
        origin: spec.origin.to_owned(),
        agent_type: (spec.origin == "subagent").then(|| "Explore".to_owned()),
        model_requested: spec.asked.to_owned(),
        model_effective: spec.answered.to_owned(),
        usage: tokens(spec.input),
        message_key: spec.key.map(str::to_owned),
        proxy_overhead_us: spec.overhead_us,
        status: 200,
        miss_cause: spec.miss.map(str::to_owned),
        ..ProxyRequestRecord::default()
    }
}

/// Five messages: two seen by both (one of them rewritten to haiku), one only in the transcripts
/// (the proxy was off), one only at the proxy (no message id) and one auxiliary request.
fn fixture() -> (Vec<UsageRecord>, Vec<ProxyRequestRecord>) {
    let rows = vec![
        transcript("m1:r1", 10, OPUS, tokens(1_000_000), false),
        transcript("m2:r2", 20, SONNET, tokens(1_000_000), true),
        transcript("m3:r3", 30, SONNET, tokens(2_000_000), false),
    ];
    let spec = |id, key, ts, origin, asked, answered, input, miss, overhead_us| {
        proxied(&Spec {
            id,
            key,
            ts,
            origin,
            asked,
            answered,
            input,
            miss,
            overhead_us,
        })
    };
    let requests = vec![
        spec(
            "p1",
            Some("m1:r1"),
            10,
            "main",
            OPUS,
            OPUS,
            1_000_000,
            Some("ttl_expired"),
            400,
        ),
        spec(
            "p2",
            Some("m2:r2"),
            20,
            "subagent",
            SONNET,
            HAIKU,
            1_000_000,
            None,
            600,
        ),
        spec(
            "p4", None, 40, "subagent", SONNET, HAIKU, 3_000_000, None, 500,
        ),
        spec(
            "p5",
            Some("m5:r5"),
            50,
            "auxiliary",
            HAIKU,
            HAIKU,
            10_000,
            None,
            12_000,
        ),
    ];
    (rows, requests)
}

fn decisions() -> Vec<RoutingDecision> {
    let decision = |id: &str, ts: i64, outcome: &str, from: f64, to: f64| RoutingDecision {
        id: id.to_owned(),
        ts: Timestamp(T0 + ts),
        outcome: outcome.to_owned(),
        est_cost_from: from,
        est_cost_to: to,
        ..RoutingDecision::default()
    };
    vec![
        decision("d1", 20, "applied", 3.0, 1.0),
        decision("d2", 40, "observed", 9.0, 3.0),
    ]
}

fn stats_by(records: &[UsageRecord], group_by: GroupBy) -> tokred_metrics::Stats {
    compute_stats(&StatsInput {
        usage: records,
        prices: &prices(),
        events: &[],
        range: TimeRange::new(Timestamp(T0), Timestamp(T0 + 3_600)),
        group_by,
        now: Timestamp(T0 + 3_600),
        first_configure: None,
    })
}

#[test]
fn the_merged_source_prices_what_really_answered() {
    let (rows, requests) = fixture();
    let merged = merge_usage(&rows, &requests);
    assert_eq!(
        (
            merged.summary.matched,
            merged.summary.proxy_only,
            merged.summary.transcript_only
        ),
        (2, 2, 1)
    );

    let stats = stats_by(&merged.records, GroupBy::Model);
    // Opus 1M x 15 + haiku 1M x 1 (rewritten) + sonnet 2M x 3 (the proxy was off)
    // + haiku 3M x 1 + haiku 10k x 1.
    assert_eq!(stats.totals.messages, 5);
    assert_eq!(stats.totals.cost_usd, 25.01);
    insta::assert_json_snapshot!("merged_stats", stats);

    let by_origin = stats_by(&merged.records, GroupBy::Origin);
    insta::assert_json_snapshot!("merged_by_origin", by_origin.groups);
}

#[test]
fn the_proxy_sections() {
    let (rows, requests) = fixture();
    let merged = merge_usage(&rows, &requests);
    let suggestions = [SuggestionRecord {
        id: "sg1".to_owned(),
        ts: Timestamp(T0 + 45),
        session_id: "s1".to_owned(),
        kind: "cheaper_model".to_owned(),
        message: "The last 3 prompts were simple; sonnet would have done".to_owned(),
        shown: false,
        detail: "{}".to_owned(),
    }];
    let stats = proxy_stats(&ProxyInput {
        usage: &merged.records,
        requests: &requests,
        decisions: &decisions(),
        suggestions: &suggestions,
        prices: &prices(),
        range: TimeRange::new(Timestamp(T0), Timestamp(T0 + 3_600)),
    })
    .unwrap();
    assert!(
        stats.overhead.unwrap().over_budget,
        "one request took 12 ms and the p95 of four is the slowest"
    );
    insta::assert_json_snapshot!("proxy_stats", stats);
}
