//! `usage` and `ingest_state` repositories against a real SQLite file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokred_core::Timestamp;
use tokred_core::pricing::Usage;
use tokred_core::usage::{IngestRepo, IngestState, UsageRecord, UsageRepo};
use tokred_store::Store;

fn record(key: &str, ts: i64, subagent: bool) -> UsageRecord {
    UsageRecord {
        message_key: key.to_owned(),
        session_id: "s1".to_owned(),
        project: "-p".to_owned(),
        ts: Timestamp(ts),
        model: "claude-opus-4-1-20250805".to_owned(),
        usage: Usage {
            input: 1,
            output: 2,
            cache_write: 3,
            cache_write_1h: 0,
            cache_read: 4,
        },
        is_subagent: subagent,
    }
}

#[test]
fn insert_new_is_idempotent_on_message_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("t.db")).unwrap();
    assert_eq!(store.earliest_usage().unwrap(), None);

    let batch = vec![record("a:1", 30, false), record("b:2", 10, true)];
    assert_eq!(store.insert_new(&batch).unwrap(), 2);
    // Same keys again plus one new row: only the new one counts.
    let again = vec![record("a:1", 99, false), record("c:3", 20, false)];
    assert_eq!(store.insert_new(&again).unwrap(), 1);
    assert_eq!(store.insert_new(&[]).unwrap(), 0);

    assert_eq!(store.earliest_usage().unwrap(), Some(Timestamp(10)));
    let rows = store.usage_between(Timestamp(0), Timestamp(1000)).unwrap();
    let keys: Vec<_> = rows.iter().map(|r| r.message_key.as_str()).collect();
    assert_eq!(
        keys,
        ["b:2", "c:3", "a:1"],
        "ordered by ts, first write wins"
    );
    assert!(rows[0].is_subagent && !rows[1].is_subagent);
    assert_eq!(rows[2].ts, Timestamp(30));
    assert_eq!(rows[2].usage.cache_read, 4);
}

#[test]
fn usage_between_is_half_open() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("t.db")).unwrap();
    store
        .insert_new(&[
            record("a", 10, false),
            record("b", 20, false),
            record("c", 30, false),
        ])
        .unwrap();
    let rows = store.usage_between(Timestamp(10), Timestamp(30)).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        store
            .usage_between(Timestamp(40), Timestamp(50))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn thousands_of_rows_insert_in_one_batch() {
    let store = Store::open_in_memory().unwrap();
    let batch: Vec<_> = (0..5000)
        .map(|i| record(&format!("k{i}"), i, false))
        .collect();
    assert_eq!(store.insert_new(&batch).unwrap(), 5000);
    assert_eq!(store.insert_new(&batch).unwrap(), 0);
    assert_eq!(
        store
            .usage_between(Timestamp(0), Timestamp(5000))
            .unwrap()
            .len(),
        5000
    );
}

#[test]
fn oversized_and_negative_counters_do_not_break_round_trips() {
    let store = Store::open_in_memory().unwrap();
    let mut big = record("big", 1, false);
    big.usage.input = u64::MAX;
    store.insert_new(&[big]).unwrap();
    let rows = store.usage_between(Timestamp(0), Timestamp(10)).unwrap();
    assert_eq!(
        rows[0].usage.input,
        i64::MAX as u64,
        "saturates instead of failing"
    );

    store
        .save_ingest_state(&IngestState {
            file: "f".into(),
            offset: u64::MAX,
            mtime: 1,
        })
        .unwrap();
    assert_eq!(
        store.ingest_state("f").unwrap().unwrap().offset,
        i64::MAX as u64
    );
}

#[test]
fn corrupt_negative_counters_read_as_zero() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    let store = Store::open(&db).unwrap();
    store.insert_new(&[record("a", 1, false)]).unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute("UPDATE usage SET input = -5", []).unwrap();
    let rows = store.usage_between(Timestamp(0), Timestamp(10)).unwrap();
    assert_eq!(rows[0].usage.input, 0);
}

#[test]
fn ingest_state_round_trip_and_update() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("t.db")).unwrap();
    assert_eq!(store.ingest_state("p/s.jsonl").unwrap(), None);
    let mut state = IngestState {
        file: "p/s.jsonl".into(),
        offset: 120,
        mtime: 7,
    };
    store.save_ingest_state(&state).unwrap();
    assert_eq!(
        store.ingest_state("p/s.jsonl").unwrap(),
        Some(state.clone())
    );
    state.offset = 300;
    state.mtime = 9;
    store.save_ingest_state(&state).unwrap();
    assert_eq!(store.ingest_state("p/s.jsonl").unwrap(), Some(state));
}

#[test]
fn backend_failures_surface_as_store_errors() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("t.db");
    let store = Store::open(&db).unwrap();
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("DROP TABLE usage; DROP TABLE ingest_state;")
        .unwrap();
    assert!(store.insert_new(&[record("a", 1, false)]).is_err());
    assert!(store.usage_between(Timestamp(0), Timestamp(1)).is_err());
    assert!(store.earliest_usage().is_err());
    assert!(store.ingest_state("f").is_err());
    let state = IngestState {
        file: "f".into(),
        offset: 1,
        mtime: 1,
    };
    assert!(store.save_ingest_state(&state).is_err());
}
