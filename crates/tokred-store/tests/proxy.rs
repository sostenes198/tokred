//! `proxy_requests` repository against a real SQLite file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokred_core::Timestamp;
use tokred_core::pricing::Usage;
use tokred_core::proxy::{HookNotes, ProxyRequestRecord};
use tokred_store::Store;

fn record(id: &str, ts: i64) -> ProxyRequestRecord {
    ProxyRequestRecord {
        id: id.to_owned(),
        ts: Timestamp(ts),
        session_id: "s1".to_owned(),
        agent_id: Some("a1".to_owned()),
        agent_type: Some("Explore".to_owned()),
        prompt_id: Some("p1".to_owned()),
        origin: "subagent".to_owned(),
        model_requested: "claude-sonnet-5-5".to_owned(),
        model_effective: "claude-haiku-5-5".to_owned(),
        usage: Usage {
            input: 10,
            output: 20,
            cache_write: 30,
            cache_write_1h: 25,
            cache_read: 40,
        },
        thinking_tokens: 7,
        message_key: Some("msg_1:req_1".to_owned()),
        bytes_in: 1000,
        bytes_out: 990,
        upstream_ttfb_ms: 120,
        total_ms: 900,
        proxy_overhead_us: 350,
        status: 200,
        notes: HookNotes {
            decision_id: Some("d1".to_owned()),
            model_rewritten: Some("claude-haiku-5-5".to_owned()),
            blocks_compressed: 3,
        },
        miss_cause: Some("idle_ttl".to_owned()),
        tools_hash: "t".repeat(64),
        system_hash: "s".repeat(64),
        stop_reason: Some("tool_use".to_owned()),
        tools: vec!["Read".to_owned(), "Grep".to_owned()],
        cache_ttl: Some("1h".to_owned()),
        routing_mode: Some("observe".to_owned()),
    }
}

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("t.db")).unwrap();
    (dir, store)
}

#[test]
fn a_request_round_trips_with_its_notes() {
    let (_dir, store) = open();
    let full = record("r1", 100);
    assert!(store.insert_proxy_request(&full).unwrap());
    let bare = ProxyRequestRecord {
        id: "r2".to_owned(),
        ts: Timestamp(101),
        status: 429,
        ..ProxyRequestRecord::default()
    };
    assert!(store.insert_proxy_request(&bare).unwrap());

    let rows = store
        .proxy_requests_between(Timestamp(0), Timestamp(1000))
        .unwrap();
    assert_eq!(rows, vec![full, bare]);
}

#[test]
fn inserting_the_same_id_twice_keeps_the_first_row() {
    let (_dir, store) = open();
    assert!(store.insert_proxy_request(&record("r1", 100)).unwrap());
    let mut changed = record("r1", 100);
    changed.status = 500;
    assert!(!store.insert_proxy_request(&changed).unwrap());
    let rows = store
        .proxy_requests_between(Timestamp(0), Timestamp(1000))
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, 200);
}

#[test]
fn huge_counters_saturate_instead_of_failing() {
    let (_dir, store) = open();
    let mut big = record("r1", 100);
    big.usage.input = u64::MAX;
    store.insert_proxy_request(&big).unwrap();
    let rows = store
        .proxy_requests_between(Timestamp(0), Timestamp(1000))
        .unwrap();
    assert_eq!(rows[0].usage.input, i64::MAX.unsigned_abs());
}

#[test]
fn the_range_is_half_open_and_ordered() {
    let (_dir, store) = open();
    for (id, ts) in [("c", 300), ("a", 100), ("b", 200), ("b2", 200)] {
        store.insert_proxy_request(&record(id, ts)).unwrap();
    }
    let ids: Vec<String> = store
        .proxy_requests_between(Timestamp(100), Timestamp(300))
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, ["a", "b", "b2"]);
}

#[test]
fn pruning_removes_rows_older_than_the_retention() {
    let (_dir, store) = open();
    let day = 86_400;
    let now = 100 * day;
    store
        .insert_proxy_request(&record("old", now - 91 * day))
        .unwrap();
    store
        .insert_proxy_request(&record("edge", now - 90 * day))
        .unwrap();
    store.insert_proxy_request(&record("new", now - 1)).unwrap();

    assert_eq!(store.prune_proxy_requests(Timestamp(now), 90).unwrap(), 1);
    assert_eq!(store.prune_proxy_requests(Timestamp(now), 90).unwrap(), 0);
    let ids: Vec<String> = store
        .proxy_requests_between(Timestamp(0), Timestamp(now + 1))
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, ["edge", "new"]);
    assert_eq!(store.prune_proxy_requests(Timestamp(now), 0).unwrap(), 2);
}

#[test]
fn rows_written_before_the_richer_fields_stay_readable() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("old.db");
    drop(Store::open(&db).unwrap());
    // A row as version 4 of the schema wrote it: none of the v6 columns.
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute(
        "INSERT INTO proxy_requests(id, ts, session_id, origin, model_requested, model_effective, \
         input, output, cache_write, cache_write_1h, cache_read, thinking, bytes_in, bytes_out, \
         upstream_ttfb_ms, total_ms, proxy_overhead_us, status, tools_hash, system_hash) \
         VALUES ('old', 5, 's', 'main', 'm', 'm', 1, 2, 3, 0, 4, 0, 1, 1, 1, 1, 1, 200, 't', 's')",
        [],
    )
    .unwrap();
    drop(conn);
    let store = Store::open(&db).unwrap();
    let rows = store
        .proxy_requests_between(Timestamp(0), Timestamp(10))
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].stop_reason, None);
    assert!(rows[0].tools.is_empty());
    assert_eq!(rows[0].cache_ttl, None);
    assert_eq!(rows[0].routing_mode, None);
}
