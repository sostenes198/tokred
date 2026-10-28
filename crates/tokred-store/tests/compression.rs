//! Integration tests of the compression and shadow-mode repositories against a real SQLite file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokred_core::Timestamp;
use tokred_core::compression::{CompressedBlock, ShadowRecord, ShadowSavings, StoredResult};
use tokred_store::Store;

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("tokred.db")).unwrap();
    (dir, store)
}

fn stored(id: &str, session: &str, expires_at: i64) -> StoredResult {
    StoredResult {
        id: id.to_owned(),
        session_id: session.to_owned(),
        tool: "Bash".to_owned(),
        original_hash: format!("{id}-hash"),
        content: "line 1\nlínea 2 ✓\n".to_owned(),
        lines: 2,
        bytes: 21,
        created_at: Timestamp(100),
        expires_at: Timestamp(expires_at),
    }
}

fn block(conversation: &str, hash: &str, replacement: &str, ts: i64) -> CompressedBlock {
    CompressedBlock {
        session_id: conversation.to_owned(),
        original_hash: hash.to_owned(),
        replacement: replacement.to_owned(),
        rules_version: "r1-h40-t20".to_owned(),
        ts: Timestamp(ts),
    }
}

fn shadow(ts: i64, session: &str, kind: &str, key: &str, bytes: u64) -> ShadowRecord {
    ShadowRecord {
        ts: Timestamp(ts),
        session_id: session.to_owned(),
        kind: kind.to_owned(),
        key: key.to_owned(),
        bytes_saved: bytes,
        est_tokens_saved: bytes / 4,
        rules_version: "r1".to_owned(),
    }
}

#[test]
fn stored_results_round_trip_and_expire() {
    let (_dir, store) = open();
    let result = stored("abc", "s1", 1000);
    store.put_stored_result(&result).unwrap();

    assert_eq!(
        store.stored_result("abc", Timestamp(999)).unwrap(),
        Some(result)
    );
    assert_eq!(store.stored_result("abc", Timestamp(1000)).unwrap(), None);
    assert_eq!(store.stored_result("nope", Timestamp(0)).unwrap(), None);
}

#[test]
fn the_same_original_keeps_its_content_and_the_later_expiry() {
    let (_dir, store) = open();
    store.put_stored_result(&stored("abc", "s1", 1000)).unwrap();
    let mut again = stored("abc", "s2", 5000);
    again.content = "ignored".to_owned();
    store.put_stored_result(&again).unwrap();
    // An earlier expiry never shortens the retention.
    store.put_stored_result(&stored("abc", "s3", 10)).unwrap();

    let kept = store
        .stored_result("abc", Timestamp(4000))
        .unwrap()
        .unwrap();
    assert_eq!(kept.session_id, "s1");
    assert_eq!(kept.content, "line 1\nlínea 2 ✓\n");
    assert_eq!(kept.expires_at, Timestamp(5000));
}

#[test]
fn pruning_deletes_only_expired_results() {
    let (_dir, store) = open();
    store.put_stored_result(&stored("old", "s", 100)).unwrap();
    store.put_stored_result(&stored("new", "s", 300)).unwrap();

    assert_eq!(store.prune_stored_results(Timestamp(100)).unwrap(), 1);
    assert_eq!(store.prune_stored_results(Timestamp(100)).unwrap(), 0);
    assert!(store.stored_result("new", Timestamp(0)).unwrap().is_some());
}

#[test]
fn compressed_blocks_keep_the_first_bytes_sent() {
    let (_dir, store) = open();
    assert!(
        store
            .insert_compressed_block(&block("s1", "h1", "first", 10))
            .unwrap()
    );
    assert!(
        !store
            .insert_compressed_block(&block("s1", "h1", "second", 20))
            .unwrap()
    );
    store
        .insert_compressed_block(&block("s1", "h0", "other", 5))
        .unwrap();
    store
        .insert_compressed_block(&block("s1/agent", "h1", "subagent", 30))
        .unwrap();

    let blocks = store.compressed_blocks("s1").unwrap();
    assert_eq!(
        blocks,
        [
            block("s1", "h0", "other", 5),
            block("s1", "h1", "first", 10)
        ]
    );
    assert_eq!(
        store.compressed_blocks("s1/agent").unwrap(),
        [block("s1/agent", "h1", "subagent", 30)]
    );
    assert!(store.compressed_blocks("none").unwrap().is_empty());
}

#[test]
fn recent_blocks_seed_every_conversation_and_old_ones_can_be_pruned() {
    let (_dir, store) = open();
    for (conversation, hash, ts) in [("a", "h1", 10), ("b", "h2", 20), ("a", "h3", 30)] {
        store
            .insert_compressed_block(&block(conversation, hash, "r", ts))
            .unwrap();
    }

    let recent = store.recent_compressed_blocks(Timestamp(20)).unwrap();
    assert_eq!(
        recent,
        [block("b", "h2", "r", 20), block("a", "h3", "r", 30)]
    );

    assert_eq!(store.prune_compressed_blocks(Timestamp(20)).unwrap(), 1);
    assert_eq!(
        store.recent_compressed_blocks(Timestamp(0)).unwrap().len(),
        2
    );
}

#[test]
fn shadow_savings_aggregate_per_kind_and_key_within_the_window() {
    let (_dir, store) = open();
    let records = [
        shadow(10, "s1", "compression", "Bash", 4000),
        shadow(20, "s2", "compression", "Bash", 8000),
        shadow(30, "s1", "compression", "Read", 40_000),
        shadow(40, "s1", "response_cache", "hash", 100),
        shadow(50, "s1", "response_cache", "hash", 100),
        shadow(99, "s3", "compression", "Bash", 1),
    ];
    assert_eq!(store.insert_shadow_records(&records).unwrap(), 6);
    assert_eq!(store.insert_shadow_records(&[]).unwrap(), 0);

    let rows = store.shadow_savings(Timestamp(10), Timestamp(99)).unwrap();
    assert_eq!(
        rows,
        [
            ShadowSavings {
                kind: "compression".to_owned(),
                key: "Read".to_owned(),
                events: 1,
                sessions: 1,
                bytes_saved: 40_000,
                est_tokens_saved: 10_000,
            },
            ShadowSavings {
                kind: "compression".to_owned(),
                key: "Bash".to_owned(),
                events: 2,
                sessions: 2,
                bytes_saved: 12_000,
                est_tokens_saved: 3000,
            },
            ShadowSavings {
                kind: "response_cache".to_owned(),
                key: "hash".to_owned(),
                events: 2,
                sessions: 1,
                bytes_saved: 200,
                est_tokens_saved: 50,
            },
        ]
    );
    assert!(
        store
            .shadow_savings(Timestamp(100), Timestamp(200))
            .unwrap()
            .is_empty()
    );

    assert_eq!(store.prune_shadow_savings(Timestamp(40)).unwrap(), 3);
    assert_eq!(
        store
            .shadow_savings(Timestamp(0), Timestamp(1000))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn a_corrupt_blob_still_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokred.db");
    let store = Store::open(&path).unwrap();
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            "INSERT INTO stored_results VALUES ('bad', 's', 't', 'h', x'61ff62', -1, 3, 1, 99);",
        )
        .unwrap();
    let read = store.stored_result("bad", Timestamp(0)).unwrap().unwrap();
    assert_eq!(read.content, "a\u{fffd}b");
    assert_eq!(read.lines, 0, "a negative counter reads as zero");
}

#[test]
fn repositories_report_backend_errors() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tokred.db");
    let store_broken = Store::open(&path).unwrap();
    // Dropping the tables makes every statement fail, which exercises the error paths.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            "DROP TABLE stored_results; DROP TABLE compressed_blocks; DROP TABLE shadow_savings;",
        )
        .unwrap();
    assert!(
        store_broken
            .put_stored_result(&stored("x", "s", 1))
            .is_err()
    );
    assert!(store_broken.stored_result("x", Timestamp(0)).is_err());
    assert!(store_broken.prune_stored_results(Timestamp(0)).is_err());
    assert!(
        store_broken
            .insert_compressed_block(&block("s", "h", "r", 1))
            .is_err()
    );
    assert!(store_broken.compressed_blocks("s").is_err());
    assert!(store_broken.recent_compressed_blocks(Timestamp(0)).is_err());
    assert!(store_broken.prune_compressed_blocks(Timestamp(0)).is_err());
    assert!(
        store_broken
            .insert_shadow_records(&[shadow(1, "s", "k", "key", 1)])
            .is_err()
    );
    assert!(
        store_broken
            .shadow_savings(Timestamp(0), Timestamp(1))
            .is_err()
    );
    assert!(store_broken.prune_shadow_savings(Timestamp(0)).is_err());
}
