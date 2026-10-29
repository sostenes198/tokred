//! `captured_turns` and the dashboard queries against a real SQLite file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokred_core::Timestamp;
use tokred_core::dashboard::{ACTIVE_WINDOW_SECS, CapturedTurn};
use tokred_core::pricing::Usage;
use tokred_core::proxy::{HookNotes, ProxyRequestRecord};
use tokred_core::routing::{RoutingDecision, RoutingRepo};
use tokred_store::Store;

const DAY: i64 = 86_400;

fn request(id: &str, session: &str, ts: i64, model: &str) -> ProxyRequestRecord {
    ProxyRequestRecord {
        id: id.to_owned(),
        ts: Timestamp(ts),
        session_id: session.to_owned(),
        origin: "main".to_owned(),
        model_requested: "claude-sonnet-5-5".to_owned(),
        model_effective: model.to_owned(),
        usage: Usage {
            input: 10,
            output: 20,
            cache_write: 30,
            cache_write_1h: 5,
            cache_read: 40,
        },
        status: 200,
        ..ProxyRequestRecord::default()
    }
}

fn captured(id: &str, session: &str, ts: i64, content: bool) -> CapturedTurn {
    CapturedTurn {
        request_id: id.to_owned(),
        session_id: session.to_owned(),
        agent_id: None,
        ts: Timestamp(ts),
        stop_reason: Some("tool_use".to_owned()),
        tools_json: r#"["Read"]"#.to_owned(),
        sent_json: content.then(|| r#"[{"role":"user","text":"hi"}]"#.to_owned()),
        response_json: content
            .then(|| r#"{"text":"ok","thinking":null,"tool_uses":[]}"#.to_owned()),
    }
}

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("t.db")).unwrap();
    (dir, store)
}

#[test]
fn migration_v5_creates_the_table_on_an_existing_v4_database() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    drop(Store::open(&db).unwrap());
    let conn = rusqlite::Connection::open(&db).unwrap();
    // What a version 4 database looks like: no `captured_turns` (v5) and no richer request
    // columns (v6).
    conn.execute_batch(
        "DROP TABLE captured_turns; \
         ALTER TABLE proxy_requests DROP COLUMN stop_reason; \
         ALTER TABLE proxy_requests DROP COLUMN tools; \
         ALTER TABLE proxy_requests DROP COLUMN cache_ttl; \
         ALTER TABLE proxy_requests DROP COLUMN routing_mode; \
         PRAGMA user_version = 4;",
    )
    .unwrap();
    drop(conn);
    let store = Store::open(&db).unwrap();
    assert!(store.schema_version().unwrap() >= 5);
    assert!(
        store
            .table_names()
            .unwrap()
            .iter()
            .any(|name| name == "captured_turns")
    );
    store
        .insert_captured_turn(&captured("r", "s", 1, true))
        .unwrap();
}

#[test]
fn a_captured_turn_is_replaced_not_duplicated() {
    let (_dir, store) = open();
    store
        .insert_proxy_request(&request("r1", "s1", 100, "m"))
        .unwrap();
    store
        .insert_captured_turn(&captured("r1", "s1", 100, false))
        .unwrap();
    store
        .insert_captured_turn(&captured("r1", "s1", 100, true))
        .unwrap();
    let (turns, truncated) = store.session_turns("s1", 10).unwrap();
    assert!(!truncated);
    assert_eq!(turns.len(), 1);
    let row = turns[0].captured.clone().unwrap();
    assert_eq!(row.stop_reason.as_deref(), Some("tool_use"));
    assert_eq!(row.tools_json, r#"["Read"]"#);
    assert!(row.sent_json.unwrap().contains("hi"));
    assert!(row.response_json.unwrap().contains("ok"));
}

#[test]
fn turns_come_back_oldest_first_in_write_order_with_their_extras() {
    let (_dir, store) = open();
    // Same second: the ids sort the other way round, the write order wins.
    for (id, ts) in [("zz", 100), ("mm", 100), ("aa", 100), ("late", 101)] {
        store
            .insert_proxy_request(&request(id, "s1", ts, "m"))
            .unwrap();
    }
    store
        .insert_proxy_request(&request("other", "s2", 100, "m"))
        .unwrap();
    let mut routed = request("routed", "s1", 102, "m");
    routed.notes = HookNotes {
        decision_id: Some("d1".to_owned()),
        ..HookNotes::default()
    };
    store.insert_proxy_request(&routed).unwrap();
    store
        .insert_decision(&RoutingDecision {
            id: "d1".to_owned(),
            session_id: "s1".to_owned(),
            outcome: "applied".to_owned(),
            reason: "read-only tools".to_owned(),
            ..RoutingDecision::default()
        })
        .unwrap();
    store
        .insert_captured_turn(&captured("late", "s1", 101, false))
        .unwrap();

    let (turns, truncated) = store.session_turns("s1", 10).unwrap();
    assert!(!truncated);
    let ids: Vec<&str> = turns.iter().map(|t| t.record.id.as_str()).collect();
    assert_eq!(ids, ["zz", "mm", "aa", "late", "routed"]);
    assert!(turns[0].captured.is_none() && turns[0].routing.is_none());
    let late = turns[3].captured.clone().unwrap();
    assert_eq!(late.sent_json, None, "no content was recorded");
    assert_eq!(
        turns[4].routing,
        Some(("applied".to_owned(), "read-only tools".to_owned()))
    );

    let (newest, truncated) = store.session_turns("s1", 2).unwrap();
    assert!(truncated, "older turns exist");
    let ids: Vec<&str> = newest.iter().map(|t| t.record.id.as_str()).collect();
    assert_eq!(ids, ["late", "routed"]);
    assert!(store.session_turns("nobody", 5).unwrap().0.is_empty());
}

#[test]
fn a_decision_without_a_row_is_just_absent() {
    let (_dir, store) = open();
    let mut r = request("r", "s1", 1, "m");
    r.notes.decision_id = Some("gone".to_owned());
    store.insert_proxy_request(&r).unwrap();
    assert_eq!(store.session_turns("s1", 5).unwrap().0[0].routing, None);
}

#[test]
fn sessions_are_summed_per_model_newest_first() {
    let (_dir, store) = open();
    store
        .insert_proxy_request(&request("a1", "old", 100, "opus"))
        .unwrap();
    store
        .insert_proxy_request(&request("b1", "new", 200, "opus"))
        .unwrap();
    store
        .insert_proxy_request(&request("b2", "new", 210, "haiku"))
        .unwrap();
    store
        .insert_proxy_request(&request("b3", "new", 220, "opus"))
        .unwrap();
    store
        .insert_proxy_request(&request("none", "", 300, "opus"))
        .unwrap();
    store
        .insert_captured_turn(&captured("b1", "new", 200, true))
        .unwrap();
    store
        .insert_captured_turn(&captured("b2", "new", 210, false))
        .unwrap();

    let rows = store.session_model_rows(None, 10).unwrap();
    let shape: Vec<(&str, &str, u64)> = rows
        .iter()
        .map(|r| (r.session_id.as_str(), r.model.as_str(), r.requests))
        .collect();
    assert_eq!(
        shape,
        [("new", "opus", 2), ("new", "haiku", 1), ("old", "opus", 1)],
        "first model used first, the session without an id is left out"
    );
    let opus = &rows[0];
    assert_eq!(
        (opus.first_ts, opus.last_ts),
        (Timestamp(200), Timestamp(220))
    );
    assert_eq!(opus.usage.input, 20);
    assert_eq!(opus.usage.cache_write_1h, 10);
    assert_eq!(
        opus.content_turns, 1,
        "one of the session's turns has content"
    );
    assert_eq!(rows[2].content_turns, 0);

    let limited = store.session_model_rows(None, 1).unwrap();
    assert!(limited.iter().all(|r| r.session_id == "new"));
    let only = store.session_model_rows(Some("old"), 10).unwrap();
    assert_eq!(only.len(), 1);
    assert!(
        store
            .session_model_rows(Some("none-such"), 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn pruning_empties_contents_outside_the_window_and_deletes_old_rows() {
    let (_dir, store) = open();
    let now = 100 * DAY;
    // Retention: older than 30 days is deleted outright.
    store
        .insert_captured_turn(&captured("ancient", "s0", now - 40 * DAY, true))
        .unwrap();
    // Twelve ended sessions, one turn each; the lower the number, the newer.
    for n in 1..=12_i64 {
        let ts = now - ACTIVE_WINDOW_SECS - 100 - n * 10;
        store
            .insert_captured_turn(&captured(&format!("e{n}"), &format!("ended{n}"), ts, true))
            .unwrap();
    }
    // An active session.
    store
        .insert_captured_turn(&captured("act", "active", now - 5, true))
        .unwrap();

    let changed = store.prune_captured_turns(Timestamp(now), 30, 10).unwrap();
    assert_eq!(changed, 1 + 2, "one deleted, two emptied");

    let has_content = |id: &str, session: &str| {
        store
            .insert_proxy_request(&request(id, session, 1, "m"))
            .unwrap();
        let (turns, _) = store.session_turns(session, 5).unwrap();
        turns[0].captured.clone().map(|c| c.sent_json.is_some())
    };
    assert_eq!(has_content("act", "active"), Some(true));
    assert_eq!(has_content("e1", "ended1"), Some(true));
    assert_eq!(has_content("e10", "ended10"), Some(true));
    assert_eq!(has_content("e11", "ended11"), Some(false));
    assert_eq!(has_content("e12", "ended12"), Some(false));
    let emptied = store.session_turns("ended12", 5).unwrap().0[0]
        .captured
        .clone()
        .unwrap();
    assert_eq!(
        emptied.stop_reason.as_deref(),
        Some("tool_use"),
        "metadata stays"
    );
    assert_eq!(emptied.tools_json, r#"["Read"]"#);
    assert_eq!(has_content("ancient", "s0"), None, "deleted");

    assert_eq!(
        store.prune_captured_turns(Timestamp(now), 30, 10).unwrap(),
        0
    );
}

#[test]
fn a_missing_table_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    let store = Store::open(&db).unwrap();
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch("DROP TABLE captured_turns;")
        .unwrap();
    assert!(
        store
            .insert_captured_turn(&captured("r", "s", 1, true))
            .is_err()
    );
    assert!(store.session_model_rows(None, 5).is_err());
    assert!(store.session_turns("s", 5).is_err());
    assert!(store.prune_captured_turns(Timestamp(1), 1, 1).is_err());
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch("DROP TABLE proxy_requests;")
        .unwrap();
    assert!(store.session_turns("s", 5).is_err());
}
