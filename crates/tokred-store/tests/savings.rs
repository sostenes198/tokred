//! The savings figures (routing decisions, compression measurements, input tokens) and the weekly
//! schedule of the savings hint.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokred_core::Timestamp;
use tokred_core::compression::ShadowRecord;
use tokred_core::pricing::Usage;
use tokred_core::proxy::ProxyRequestRecord;
use tokred_core::routing::{RoutingDecision, RoutingRepo};
use tokred_core::savings::{SavingsRepo, Verdict};
use tokred_store::Store;

fn decision(id: &str, ts: i64, outcome: &str, from: f64, to: f64) -> RoutingDecision {
    RoutingDecision {
        id: id.to_owned(),
        ts: Timestamp(ts),
        session_id: "s1".to_owned(),
        task_class: "search".to_owned(),
        model_from: "claude-sonnet-5-5".to_owned(),
        model_to: "claude-haiku-5-5".to_owned(),
        mode: "observe".to_owned(),
        est_cost_from: from,
        est_cost_to: to,
        outcome: outcome.to_owned(),
        ..RoutingDecision::default()
    }
}

#[test]
fn an_empty_store_has_no_figures() {
    let store = Store::open_in_memory().unwrap();
    let figures = store
        .savings_figures(Timestamp(0), Timestamp(7 * 86_400))
        .unwrap();
    assert!((figures.days - 7.0).abs() < 1e-9);
    assert_eq!(
        (
            figures.routing_decisions,
            figures.routing_changed,
            figures.compression_tokens,
            figures.input_tokens
        ),
        (0, 0, 0, 0)
    );
    assert!(figures.routing_saving_usd.abs() < f64::EPSILON);
    assert_eq!(figures.routing_verdict(), Verdict::NoData);
}

#[test]
fn the_figures_add_up_what_each_feature_would_have_saved_in_the_period() {
    let store = Store::open_in_memory().unwrap();
    // Routing: two rerouted (one applied, one observed), one kept, one outside the period.
    store
        .insert_decision(&decision("a", 100, "observed", 0.50, 0.10))
        .unwrap();
    store
        .insert_decision(&decision("b", 200, "applied", 1.00, 0.20))
        .unwrap();
    store
        .insert_decision(&decision("c", 300, "kept", 0.40, 0.40))
        .unwrap();
    store
        .insert_decision(&decision("old", 5_000, "observed", 9.0, 1.0))
        .unwrap();
    // Compression measurements: only `compression`, only inside the period.
    let shadow = |kind: &str, ts: i64, tokens: u64| ShadowRecord {
        ts: Timestamp(ts),
        session_id: "s1".to_owned(),
        kind: kind.to_owned(),
        key: "Read".to_owned(),
        bytes_saved: tokens * 4,
        est_tokens_saved: tokens,
        rules_version: "r".to_owned(),
    };
    store
        .insert_shadow_records(&[
            shadow("compression", 150, 1_000),
            shadow("compression", 250, 500),
            shadow("response_cache", 150, 9_999),
            shadow("compression", 5_000, 7_777),
        ])
        .unwrap();
    // Input tokens: new + cache read + cache write of the requests of the period.
    for (id, ts, input) in [("r1", 120, 10), ("r2", 220, 20), ("late", 5_000, 1_000)] {
        let record = ProxyRequestRecord {
            id: id.to_owned(),
            ts: Timestamp(ts),
            usage: Usage {
                input,
                cache_read: 100,
                cache_write: 5,
                ..Usage::default()
            },
            ..ProxyRequestRecord::default()
        };
        assert!(store.insert_proxy_request(&record).unwrap());
    }

    let figures = store
        .savings_figures(Timestamp(0), Timestamp(1_000))
        .unwrap();
    assert_eq!(
        (
            figures.routing_decisions,
            figures.routing_changed,
            figures.compression_tokens,
            figures.input_tokens
        ),
        (3, 2, 1_500, 10 + 100 + 5 + 20 + 100 + 5)
    );
    assert!(
        (figures.routing_saving_usd - 1.20).abs() < 1e-9,
        "{figures:?}"
    );
}

#[test]
fn the_hint_is_claimed_once_a_week() {
    let store = Store::open_in_memory().unwrap();
    let week = 7 * 86_400;
    assert!(store.claim_savings_hint(Timestamp(1_000), week).unwrap());
    assert!(
        !store.claim_savings_hint(Timestamp(1_000), week).unwrap(),
        "already claimed"
    );
    assert!(
        !store
            .claim_savings_hint(Timestamp(1_000 + week - 1), week)
            .unwrap(),
        "a second before the week is out"
    );
    assert!(
        store
            .claim_savings_hint(Timestamp(1_000 + week), week)
            .unwrap(),
        "due again"
    );
    assert!(
        !store
            .claim_savings_hint(Timestamp(1_000 + week), week)
            .unwrap()
    );
}
