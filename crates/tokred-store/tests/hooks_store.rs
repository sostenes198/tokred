//! Repositories used by the hooks (sessions, file reads, classifications, hook events, cache)
//! against a real SQLite file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rusqlite::Connection;
use tokred_core::Timestamp;
use tokred_core::cache::{CacheRepo, Tier, TierPolicy, cache_get, cache_put};
use tokred_core::hook_event::{HookEventRecord, HookEventRepo, Outcome};
use tokred_core::session::{
    Classification, ClassificationRepo, FileRead, FileReadRepo, SessionRepo, TaskClass,
};
use tokred_store::Store;

fn open() -> (tempfile::TempDir, Store, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tokred.db");
    let store = Store::open(&db).unwrap();
    (dir, store, db)
}

fn raw(db: &std::path::Path) -> Connection {
    Connection::open(db).unwrap()
}

fn read(scope: &str, offset: Option<u64>, limit: Option<u64>, ts: i64) -> FileRead {
    FileRead {
        session_id: scope.to_owned(),
        path: "/p/a.rs".to_owned(),
        hash: "h1".to_owned(),
        offset,
        limit,
        bytes: 100,
        ts: Timestamp(ts),
    }
}

#[test]
fn sessions_are_registered_touched_and_reset() {
    let (_dir, store, _) = open();
    assert_eq!(store.get_session("s").unwrap(), None);

    store
        .touch_session("s", Some("/work"), Timestamp(10))
        .unwrap();
    let first = store.get_session("s").unwrap().unwrap();
    assert_eq!(first.project.as_deref(), Some("/work"));
    assert_eq!(
        (first.started_at, first.last_seen),
        (Timestamp(10), Timestamp(10))
    );
    assert_eq!(first.last_reset_at, None);

    // A later touch without a project keeps the project and moves last_seen only.
    store.touch_session("s", None, Timestamp(20)).unwrap();
    let second = store.get_session("s").unwrap().unwrap();
    assert_eq!(second.project.as_deref(), Some("/work"));
    assert_eq!(
        (second.started_at, second.last_seen),
        (Timestamp(10), Timestamp(20))
    );

    store.mark_reset("s", Timestamp(30)).unwrap();
    let third = store.get_session("s").unwrap().unwrap();
    assert_eq!(third.last_reset_at, Some(Timestamp(30)));
    assert_eq!(third.last_seen, Timestamp(30));
    assert_eq!(third.project.as_deref(), Some("/work"));
}

#[test]
fn a_reset_registers_an_unknown_session() {
    let (_dir, store, _) = open();
    store.mark_reset("new", Timestamp(5)).unwrap();
    let session = store.get_session("new").unwrap().unwrap();
    assert_eq!(session.last_reset_at, Some(Timestamp(5)));
    assert_eq!(session.project, None);
}

#[test]
fn file_reads_match_on_the_exact_range_including_an_absent_one() {
    let (_dir, store, _) = open();
    let full = read("s", None, None, 1);
    assert_eq!(
        store.find_read("s", "/p/a.rs", "h1", None, None).unwrap(),
        None
    );

    store.record_read(&full).unwrap();
    assert_eq!(
        store.find_read("s", "/p/a.rs", "h1", None, None).unwrap(),
        Some(full.clone())
    );
    // A ranged read is a different key.
    assert_eq!(
        store
            .find_read("s", "/p/a.rs", "h1", Some(0), Some(50))
            .unwrap(),
        None
    );
    store.record_read(&read("s", Some(0), Some(50), 2)).unwrap();
    assert!(
        store
            .find_read("s", "/p/a.rs", "h1", Some(0), Some(50))
            .unwrap()
            .is_some()
    );
    // Other hash, other scope and other path do not match.
    assert_eq!(
        store.find_read("s", "/p/a.rs", "h2", None, None).unwrap(),
        None
    );
    assert_eq!(
        store
            .find_read("s/agent", "/p/a.rs", "h1", None, None)
            .unwrap(),
        None
    );
    assert_eq!(
        store.find_read("s", "/p/b.rs", "h1", None, None).unwrap(),
        None
    );
}

#[test]
fn recording_the_same_read_again_refreshes_it_instead_of_duplicating() {
    let (_dir, store, db) = open();
    store.record_read(&read("s", None, None, 1)).unwrap();
    let mut again = read("s", None, None, 9);
    again.bytes = 7;
    store.record_read(&again).unwrap();

    let found = store
        .find_read("s", "/p/a.rs", "h1", None, None)
        .unwrap()
        .unwrap();
    assert_eq!((found.ts, found.bytes), (Timestamp(9), 7));
    let rows: i64 = raw(&db)
        .query_row("SELECT COUNT(*) FROM file_reads", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn huge_byte_counts_do_not_overflow_the_database_integer() {
    let (_dir, store, _) = open();
    let mut huge = read("s", None, None, 1);
    huge.bytes = u64::MAX;
    store.record_read(&huge).unwrap();
    let found = store
        .find_read("s", "/p/a.rs", "h1", None, None)
        .unwrap()
        .unwrap();
    assert_eq!(found.bytes, i64::MAX as u64);
}

#[test]
fn the_last_classification_is_the_most_recent_one() {
    let (_dir, store, db) = open();
    assert_eq!(store.last_classification("s").unwrap(), None);
    let make = |ts, class| Classification {
        session_id: "s".to_owned(),
        ts: Timestamp(ts),
        class,
        prompt_len: 42,
    };
    store
        .insert_classification(&make(1, TaskClass::Review))
        .unwrap();
    store
        .insert_classification(&make(5, TaskClass::Implementation))
        .unwrap();
    // Same second as the previous one: the later insert wins.
    store
        .insert_classification(&make(5, TaskClass::Search))
        .unwrap();
    store
        .insert_classification(&make(3, TaskClass::Summary))
        .unwrap();

    let last = store.last_classification("s").unwrap().unwrap();
    assert_eq!(last, make(5, TaskClass::Search));
    assert_eq!(store.last_classification("other").unwrap(), None);

    // An unknown class in the table degrades to `other` instead of failing.
    raw(&db)
        .execute(
            "INSERT INTO classifications VALUES ('weird', 1, 'from-the-future', 3)",
            [],
        )
        .unwrap();
    assert_eq!(
        store.last_classification("weird").unwrap().unwrap().class,
        TaskClass::Other
    );
}

fn event(ts: i64, outcome: Outcome) -> HookEventRecord {
    HookEventRecord {
        ts: Timestamp(ts),
        session_id: "s".to_owned(),
        hook: "guard-read".to_owned(),
        decision: "deny".to_owned(),
        reason: "reread".to_owned(),
        bytes_saved: 1234,
        duration_us: 800,
        outcome,
    }
}

#[test]
fn hook_events_are_stored_and_listed_by_time_window() {
    let (_dir, store, db) = open();
    for (ts, outcome) in [
        (10, Outcome::Ok),
        (20, Outcome::Error),
        (30, Outcome::Panic),
        (40, Outcome::TimeoutGuard),
    ] {
        store.insert_event(&event(ts, outcome)).unwrap();
    }
    let all = store.events_between(Timestamp(0), Timestamp(100)).unwrap();
    assert_eq!(all.len(), 4);
    assert_eq!(all[0], event(10, Outcome::Ok));
    assert_eq!(all[3].outcome, Outcome::TimeoutGuard);

    // `from` is inclusive and `to` is exclusive.
    let window = store.events_between(Timestamp(20), Timestamp(40)).unwrap();
    assert_eq!(window.iter().map(|e| e.ts.0).collect::<Vec<_>>(), [20, 30]);

    // An unknown outcome and a negative count degrade instead of failing.
    raw(&db)
        .execute(
            "INSERT INTO hook_events VALUES (50, 's', 'x', 'd', 'r', -5, -7, 'mystery')",
            [],
        )
        .unwrap();
    let odd = store.events_between(Timestamp(50), Timestamp(51)).unwrap();
    assert_eq!(odd[0].outcome, Outcome::Error);
    assert_eq!((odd[0].bytes_saved, odd[0].duration_us), (0, 0));
}

#[test]
fn cache_entries_round_trip_and_promote_through_the_store() {
    let (_dir, store, _) = open();
    let policy = TierPolicy::default();
    assert_eq!(store.get_entry("k").unwrap(), None);

    cache_put(
        &store,
        "k",
        "repo_files",
        b"123".to_vec(),
        Timestamp(100),
        &policy,
    )
    .unwrap();
    let fresh = store.get_entry("k").unwrap().unwrap();
    assert_eq!((fresh.tier, fresh.hits), (Tier::L1, 0));
    assert_eq!(fresh.value, b"123");
    assert_eq!(fresh.expires_at, Some(Timestamp(100 + 4 * 3600)));

    for t in 1..=3 {
        cache_get(&store, "k", Timestamp(100 + t), &policy)
            .unwrap()
            .unwrap();
    }
    let promoted = store.get_entry("k").unwrap().unwrap();
    assert_eq!((promoted.tier, promoted.hits), (Tier::L2, 3));

    for t in 4..=10 {
        cache_get(&store, "k", Timestamp(100 + t), &policy)
            .unwrap()
            .unwrap();
    }
    let persistent = store.get_entry("k").unwrap().unwrap();
    assert_eq!(persistent.tier, Tier::L3);
    assert_eq!(persistent.expires_at, None);
}

#[test]
fn prune_removes_only_expired_rows_and_keeps_persistent_ones() {
    let (_dir, store, _) = open();
    let policy = TierPolicy::default();
    cache_put(&store, "old", "k", vec![], Timestamp(0), &policy).unwrap();
    cache_put(&store, "new", "k", vec![], Timestamp(10_000), &policy).unwrap();
    let mut forever = tokred_core::cache::new_entry("forever", "k", vec![], Timestamp(0), &policy);
    forever.tier = Tier::L3;
    forever.expires_at = None;
    store.put_entry(&forever).unwrap();

    assert_eq!(store.delete_expired(Timestamp(5 * 3600)).unwrap(), 1);
    let keys: Vec<_> = store
        .list_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.key)
        .collect();
    assert_eq!(keys, ["forever", "new"]);
    assert_eq!(store.delete_expired(Timestamp(5 * 3600)).unwrap(), 0);
}

#[test]
fn an_unknown_tier_in_the_table_reads_as_l1() {
    let (_dir, store, db) = open();
    raw(&db)
        .execute(
            "INSERT INTO cache_entries VALUES ('odd', 'k', x'00', 9, 99999999999, 1, 1, NULL)",
            [],
        )
        .unwrap();
    let odd = store.get_entry("odd").unwrap().unwrap();
    assert_eq!((odd.tier, odd.hits), (Tier::L1, u32::MAX));
}

#[test]
fn repository_errors_are_reported_not_panicked() {
    let (_dir, store, db) = open();
    raw(&db)
        .execute_batch(
            "DROP TABLE sessions; DROP TABLE file_reads; DROP TABLE classifications; \
             DROP TABLE hook_events; DROP TABLE cache_entries;",
        )
        .unwrap();
    let policy = TierPolicy::default();

    assert!(store.touch_session("s", None, Timestamp(1)).is_err());
    assert!(store.mark_reset("s", Timestamp(1)).is_err());
    assert!(store.get_session("s").is_err());

    assert!(store.find_read("s", "p", "h", None, None).is_err());
    assert!(store.record_read(&read("s", None, None, 1)).is_err());

    assert!(store.last_classification("s").is_err());
    let class = Classification {
        session_id: "s".to_owned(),
        ts: Timestamp(1),
        class: TaskClass::Other,
        prompt_len: 1,
    };
    assert!(store.insert_classification(&class).is_err());

    assert!(store.insert_event(&event(1, Outcome::Ok)).is_err());
    assert!(store.events_between(Timestamp(0), Timestamp(9)).is_err());

    assert!(store.get_entry("k").is_err());
    assert!(cache_put(&store, "k", "k", vec![], Timestamp(1), &policy).is_err());
    assert!(store.delete_expired(Timestamp(1)).is_err());
    assert!(store.list_entries().is_err());
}
