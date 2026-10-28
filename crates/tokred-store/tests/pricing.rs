//! `PriceRepo` on a real SQLite file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokred_core::Timestamp;
use tokred_core::pricing::{ModelPrice, PriceEntry, PriceRepo};
use tokred_store::Store;

fn entry(model: &str, input: f64, source: &str, at: i64) -> PriceEntry {
    PriceEntry {
        model: model.to_owned(),
        price: ModelPrice {
            input,
            output: input * 5.0,
            cache_write: input * 1.25,
            cache_read: input * 0.1,
        },
        source: source.to_owned(),
        updated_at: Timestamp(at),
    }
}

#[test]
fn an_empty_table_loads_nothing() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.load().unwrap().is_empty());
}

#[test]
fn rows_round_trip_sorted_by_model() {
    let store = Store::open_in_memory().unwrap();
    let rows = vec![
        entry("claude-sonnet-4-5", 3.0, "litellm", 10),
        entry("claude-haiku-4-5", 1.0, "embedded", 20),
    ];
    store.replace_all(&rows).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded, vec![rows[1].clone(), rows[0].clone()]);
}

#[test]
fn replace_all_drops_rows_that_are_gone_and_survives_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tokred.db");
    {
        let store = Store::open(&db).unwrap();
        store
            .replace_all(&[
                entry("claude-a", 1.0, "litellm", 1),
                entry("claude-b", 2.0, "litellm", 1),
            ])
            .unwrap();
        store
            .replace_all(&[entry("claude-b", 4.0, "litellm", 2)])
            .unwrap();
    }
    let store = Store::open(&db).unwrap();
    assert_eq!(
        store.load().unwrap(),
        vec![entry("claude-b", 4.0, "litellm", 2)]
    );
    store.replace_all(&[]).unwrap();
    assert!(store.load().unwrap().is_empty());
}

#[test]
fn a_duplicate_model_in_one_batch_keeps_the_last_row() {
    let store = Store::open_in_memory().unwrap();
    store
        .replace_all(&[
            entry("claude-a", 1.0, "litellm", 1),
            entry("claude-a", 2.0, "litellm", 2),
        ])
        .unwrap();
    assert_eq!(
        store.load().unwrap(),
        vec![entry("claude-a", 2.0, "litellm", 2)]
    );
}
