//! Integration tests against a real SQLite file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use tokred_core::{ManifestEntry, ManifestRepo, Timestamp};
use tokred_store::Store;

fn entry(path: &str, hash: &str) -> ManifestEntry {
    ManifestEntry {
        path: PathBuf::from(path),
        hash: hash.to_owned(),
        version: "0.1.0".to_owned(),
        installed_at: Timestamp(100),
    }
}

#[test]
fn open_creates_the_full_schema_and_is_reopenable() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tokred.db");

    let store = Store::open(&db).unwrap();
    assert_eq!(store.schema_version().unwrap(), 1);
    let tables = store.table_names().unwrap();
    for expected in [
        "cache_entries",
        "classifications",
        "file_reads",
        "hook_events",
        "ingest_state",
        "installed_files",
        "pricing",
        "sessions",
        "usage",
    ] {
        assert!(
            tables.iter().any(|t| t == expected),
            "missing table {expected}"
        );
    }
    drop(store);

    let reopened = Store::open(&db).unwrap();
    assert_eq!(reopened.schema_version().unwrap(), 1);
}

#[test]
fn manifest_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("tokred.db")).unwrap();
    let path = PathBuf::from("/c/agents/tokred-dev.md");

    assert_eq!(store.get(&path).unwrap(), None);
    store
        .upsert(&entry("/c/agents/tokred-dev.md", "aaa"))
        .unwrap();
    store
        .upsert(&entry("/c/agents/tokred-dev.md", "bbb"))
        .unwrap();
    store
        .upsert(&entry("/c/agents/tokred-qa.md", "ccc"))
        .unwrap();

    assert_eq!(store.get(&path).unwrap().unwrap().hash, "bbb");
    assert_eq!(store.list().unwrap().len(), 2);
    store.remove(&path).unwrap();
    assert_eq!(store.get(&path).unwrap(), None);
    assert_eq!(store.list().unwrap().len(), 1);
}

#[test]
fn newer_schema_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tokred.db");
    drop(Store::open(&db).unwrap());
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("PRAGMA user_version = 99;").unwrap();
    drop(conn);
    assert!(Store::open(&db).is_err());
}

#[test]
fn in_memory_store_has_the_schema() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.schema_version().unwrap(), 1);
    assert_eq!(store.table_names().unwrap().len(), 9);
}

#[test]
fn repository_errors_are_reported_not_panicked() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tokred.db");
    let store = Store::open(&db).unwrap();
    // Break the schema behind the store's back.
    let other = rusqlite::Connection::open(&db).unwrap();
    other.execute_batch("DROP TABLE installed_files;").unwrap();

    let path = PathBuf::from("/c/agents/tokred-dev.md");
    assert!(store.get(&path).is_err());
    assert!(
        store
            .upsert(&entry("/c/agents/tokred-dev.md", "aaa"))
            .is_err()
    );
    assert!(store.remove(&path).is_err());
    assert!(store.list().is_err());
}

#[test]
fn open_errors_are_readable() {
    let dir = tempfile::tempdir().unwrap();
    // A directory cannot be opened as a database file.
    let err = Store::open(dir.path()).unwrap_err();
    assert!(err.to_string().contains("sqlite"), "{err}");

    let db = dir.path().join("t.db");
    drop(Store::open(&db).unwrap());
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch("PRAGMA user_version = 99;")
        .unwrap();
    let too_new = Store::open(&db).unwrap_err();
    assert!(too_new.to_string().contains("99"), "{too_new}");
}
