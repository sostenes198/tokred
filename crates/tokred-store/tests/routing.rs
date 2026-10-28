//! `routing_decisions`: insert (upsert by id), get by id, list by time, by agent and latest.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokred_core::Timestamp;
use tokred_core::routing::{RoutingDecision, RoutingRepo};
use tokred_store::Store;

fn decision(id: &str, ts: i64, agent: Option<&str>) -> RoutingDecision {
    RoutingDecision {
        id: id.to_owned(),
        ts: Timestamp(ts),
        session_id: "s1".to_owned(),
        agent_id: agent.map(str::to_owned),
        scope: if agent.is_some() { "agent" } else { "request" }.to_owned(),
        task_class: "search".to_owned(),
        confidence: 0.85,
        layer: "tools".to_owned(),
        reason: "read-only tools only (Read)".to_owned(),
        model_from: "claude-sonnet-5-5".to_owned(),
        model_to: "claude-haiku-5-5".to_owned(),
        mode: "observe".to_owned(),
        est_cost_from: 0.0123,
        est_cost_to: 0.0006,
        rules_version: "routing-1+catalog-2026-10-28".to_owned(),
        outcome: "observed".to_owned(),
        detail: r#"{"tier":"cheap"}"#.to_owned(),
    }
}

#[test]
fn a_decision_round_trips_and_an_update_replaces_it() {
    let store = Store::open_in_memory().unwrap();
    let original = decision("rd-1", 100, Some("a1"));
    store.insert_decision(&original).unwrap();
    assert_eq!(store.decision("rd-1").unwrap(), Some(original.clone()));
    assert_eq!(store.decision("missing").unwrap(), None);

    // The same decision re-sent with a new outcome (a stopped rewrite) replaces the row.
    let stopped = RoutingDecision {
        outcome: "fallback".to_owned(),
        reason: "stopped".to_owned(),
        detail: String::new(),
        ..original
    };
    store.insert_decision(&stopped).unwrap();
    let stored = store.decision("rd-1").unwrap().unwrap();
    assert_eq!(stored.outcome, "fallback");
    assert_eq!(stored.detail, "{}", "an empty detail is stored as {{}}");
    assert_eq!(
        store
            .decisions_between(Timestamp(0), Timestamp(1000))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn decisions_are_listed_by_time_by_agent_and_latest_first() {
    let store = Store::open_in_memory().unwrap();
    for (id, ts, agent) in [
        ("rd-3", 300, Some("a1")),
        ("rd-1", 100, Some("a1")),
        ("rd-2", 200, None),
        ("rd-4", 400, Some("a2")),
    ] {
        store.insert_decision(&decision(id, ts, agent)).unwrap();
    }
    let ids = |list: Vec<RoutingDecision>| list.into_iter().map(|d| d.id).collect::<Vec<_>>();

    assert_eq!(
        ids(store
            .decisions_between(Timestamp(100), Timestamp(400))
            .unwrap()),
        ["rd-1", "rd-2", "rd-3"]
    );
    assert_eq!(
        ids(store.decisions_for_agent("s1", "a1").unwrap()),
        ["rd-1", "rd-3"]
    );
    assert!(store.decisions_for_agent("s2", "a1").unwrap().is_empty());
    assert_eq!(ids(store.recent_decisions(2).unwrap()), ["rd-4", "rd-3"]);
    assert_eq!(store.recent_decisions(usize::MAX).unwrap().len(), 4);
    let auxiliary = store.decision("rd-2").unwrap().unwrap();
    assert_eq!(
        (auxiliary.agent_id, auxiliary.scope.as_str()),
        (None, "request")
    );
}
