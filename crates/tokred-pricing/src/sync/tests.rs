use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use tokred_core::pricing::{ModelPrice, PriceEntry, SourceError};

use super::*;

const DAY: i64 = 86_400;

struct FakeClock(Cell<i64>);

impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.get())
    }
}

#[derive(Default)]
struct FakeMeta {
    map: RefCell<HashMap<String, String>>,
    broken: Cell<bool>,
}

impl FakeMeta {
    fn with(pairs: &[(&str, &str)]) -> Self {
        let meta = Self::default();
        for (key, value) in pairs {
            meta.map
                .borrow_mut()
                .insert((*key).to_owned(), (*value).to_owned());
        }
        meta
    }

    fn get(&self, key: &str) -> Option<String> {
        self.map.borrow().get(key).cloned()
    }
}

impl MetaRepo for FakeMeta {
    fn meta_get(&self, key: &str) -> Result<Option<String>, StoreError> {
        if self.broken.get() {
            return Err(StoreError::backend(std::io::Error::other("meta down")));
        }
        Ok(self.get(key))
    }

    fn meta_set(&self, key: &str, value: &str) -> Result<(), StoreError> {
        if self.broken.get() {
            return Err(StoreError::backend(std::io::Error::other("meta down")));
        }
        self.map
            .borrow_mut()
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }
}

#[derive(Default)]
struct FakeRepo {
    rows: RefCell<Vec<PriceEntry>>,
    fail_load: Cell<bool>,
    fail_replace: Cell<bool>,
    replaced: Cell<u32>,
}

impl PriceRepo for FakeRepo {
    fn replace_all(&self, entries: &[PriceEntry]) -> Result<(), StoreError> {
        if self.fail_replace.get() {
            return Err(StoreError::backend(std::io::Error::other("disk full")));
        }
        self.replaced.set(self.replaced.get() + 1);
        *self.rows.borrow_mut() = entries.to_vec();
        Ok(())
    }

    fn load(&self) -> Result<Vec<PriceEntry>, StoreError> {
        if self.fail_load.get() {
            return Err(StoreError::backend(std::io::Error::other("unreadable")));
        }
        Ok(self.rows.borrow().clone())
    }
}

struct FakeSource(Result<Vec<PriceEntry>, String>);

impl PriceSource for FakeSource {
    fn fetch(&self) -> Result<Vec<PriceEntry>, SourceError> {
        self.0.clone().map_err(SourceError::Invalid)
    }
}

fn entry(model: &str, input: f64, updated_at: i64) -> PriceEntry {
    PriceEntry {
        model: model.to_owned(),
        price: ModelPrice {
            input,
            output: input * 5.0,
            cache_write: input * 1.25,
            cache_write_1h: input * 2.0,
            cache_read: input * 0.1,
        },
        source: "fake".to_owned(),
        updated_at: Timestamp(updated_at),
    }
}

#[test]
fn a_successful_sync_replaces_the_table_and_records_the_time() {
    let repo = FakeRepo::default();
    repo.rows.borrow_mut().push(entry("claude-old", 1.0, 1));
    let meta = FakeMeta::default();
    let clock = FakeClock(Cell::new(1_000));
    let source = FakeSource(Ok(vec![
        entry("claude-a", 3.0, 0),
        entry("claude-b", 5.0, 0),
    ]));

    let outcome = sync_prices(&source, &repo, &meta, &clock);

    assert_eq!(outcome, SyncOutcome::Synced { models: 2 });
    let rows = repo.load().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter().all(|r| r.updated_at == Timestamp(1_000)),
        "restamped"
    );
    assert_eq!(meta.get(PRICE_SYNCED_AT).as_deref(), Some("1000"));
    assert_eq!(meta.get(PRICE_SYNC_ATTEMPT_AT).as_deref(), Some("1000"));
}

#[test]
fn a_failed_fetch_keeps_the_table_and_reports_why() {
    let repo = FakeRepo::default();
    repo.rows.borrow_mut().push(entry("claude-old", 1.0, 1));
    let meta = FakeMeta::default();
    let clock = FakeClock(Cell::new(2_000));

    let outcome = sync_prices(&FakeSource(Err("offline".into())), &repo, &meta, &clock);

    let SyncOutcome::Failed {
        reason,
        seeded_fallback,
    } = outcome
    else {
        panic!("expected a failure, got {outcome:?}");
    };
    assert!(reason.contains("offline"), "{reason}");
    assert!(!seeded_fallback, "a non-empty table is left alone");
    assert_eq!(repo.load().unwrap().len(), 1);
    assert_eq!(meta.get(PRICE_SYNCED_AT), None, "no success recorded");
    assert_eq!(meta.get(PRICE_SYNC_ATTEMPT_AT).as_deref(), Some("2000"));
}

#[test]
fn a_failed_first_sync_seeds_the_embedded_fallback() {
    let repo = FakeRepo::default();
    let outcome = sync_prices(
        &FakeSource(Err("offline".into())),
        &repo,
        &FakeMeta::default(),
        &FakeClock(Cell::new(5)),
    );
    assert!(matches!(
        outcome,
        SyncOutcome::Failed {
            seeded_fallback: true,
            ..
        }
    ));
    let rows = repo.load().unwrap();
    assert!(rows.len() > 10);
    assert!(
        rows.iter()
            .all(|r| r.source == "embedded" && r.updated_at == Timestamp(5))
    );
}

#[test]
fn an_empty_answer_never_wipes_the_table() {
    let repo = FakeRepo::default();
    repo.rows.borrow_mut().push(entry("claude-old", 1.0, 1));
    let outcome = sync_prices(
        &FakeSource(Ok(Vec::new())),
        &repo,
        &FakeMeta::default(),
        &FakeClock(Cell::new(5)),
    );
    let SyncOutcome::Failed { reason, .. } = outcome else {
        panic!("expected a failure");
    };
    assert!(reason.contains("no models"));
    assert_eq!(repo.load().unwrap().len(), 1);
}

#[test]
fn store_failures_are_reported_not_hidden() {
    let source = FakeSource(Ok(vec![entry("claude-a", 3.0, 0)]));
    let repo = FakeRepo::default();
    repo.fail_replace.set(true);
    let outcome = sync_prices(
        &source,
        &repo,
        &FakeMeta::default(),
        &FakeClock(Cell::new(5)),
    );
    let SyncOutcome::Failed {
        reason,
        seeded_fallback,
    } = outcome
    else {
        panic!("expected a failure");
    };
    assert!(reason.contains("disk full"), "{reason}");
    assert!(
        !seeded_fallback,
        "seeding fails too, and that is reported as false"
    );

    // A broken meta store does not turn a good sync into a failure.
    let meta = FakeMeta::default();
    meta.broken.set(true);
    let repo = FakeRepo::default();
    assert_eq!(
        sync_prices(&source, &repo, &meta, &FakeClock(Cell::new(5))),
        SyncOutcome::Synced { models: 1 }
    );

    // An unreadable table is not seeded either.
    let repo = FakeRepo::default();
    repo.fail_load.set(true);
    let failing = FakeSource(Err("offline".into()));
    let outcome = sync_prices(
        &failing,
        &repo,
        &FakeMeta::default(),
        &FakeClock(Cell::new(5)),
    );
    assert!(matches!(
        outcome,
        SyncOutcome::Failed {
            seeded_fallback: false,
            ..
        }
    ));
    assert_eq!(repo.replaced.get(), 0);
}

#[test]
fn load_table_returns_stored_rows() {
    let repo = FakeRepo::default();
    repo.rows
        .borrow_mut()
        .push(entry("claude-sonnet-x", 3.0, 1));
    let table = load_table(&repo, &FakeClock(Cell::new(9)));
    assert_eq!(table.entries().len(), 1);
    assert_eq!(repo.replaced.get(), 0, "nothing was seeded");
}

#[test]
fn load_table_seeds_the_fallback_when_empty() {
    let repo = FakeRepo::default();
    let table = load_table(&repo, &FakeClock(Cell::new(9)));
    assert!(table.resolve("claude-opus-4-1-20250805").is_some());
    assert_eq!(repo.replaced.get(), 1);
    assert_eq!(repo.load().unwrap().len(), table.entries().len());

    // Persisting is best effort.
    let repo = FakeRepo::default();
    repo.fail_replace.set(true);
    assert!(
        !load_table(&repo, &FakeClock(Cell::new(9)))
            .entries()
            .is_empty()
    );
}

#[test]
fn load_table_falls_back_to_memory_when_the_store_is_unreadable() {
    let repo = FakeRepo::default();
    repo.fail_load.set(true);
    let table = load_table(&repo, &FakeClock(Cell::new(9)));
    assert!(table.resolve("claude-sonnet-4-5").is_some());
    assert_eq!(repo.replaced.get(), 0);
}

#[test]
fn the_sync_runs_at_most_once_a_day() {
    let clock = FakeClock(Cell::new(10 * DAY));
    // Never synced.
    assert!(sync_due(&FakeMeta::default(), &clock, true));
    // Disabled by config, whatever the state.
    assert!(!sync_due(&FakeMeta::default(), &clock, false));

    let stamp = |secs_ago: i64| (10 * DAY - secs_ago).to_string();
    let recent = FakeMeta::with(&[(PRICE_SYNCED_AT, &stamp(DAY - 1))]);
    assert!(!sync_due(&recent, &clock, true));
    let old = FakeMeta::with(&[(PRICE_SYNCED_AT, &stamp(DAY))]);
    assert!(sync_due(&old, &clock, true));

    // A recent failed attempt also holds the next one back.
    let attempted = FakeMeta::with(&[
        (PRICE_SYNCED_AT, &stamp(5 * DAY)),
        (PRICE_SYNC_ATTEMPT_AT, &stamp(60)),
    ]);
    assert!(!sync_due(&attempted, &clock, true));
}

#[test]
fn odd_timestamps_and_a_broken_store_are_handled() {
    let clock = FakeClock(Cell::new(10 * DAY));
    let garbage = FakeMeta::with(&[(PRICE_SYNCED_AT, "yesterday")]);
    assert!(
        sync_due(&garbage, &clock, true),
        "unparsable counts as never"
    );
    let future = FakeMeta::with(&[(PRICE_SYNCED_AT, &(11 * DAY).to_string())]);
    assert!(sync_due(&future, &clock, true), "a clock that went back");
    let broken = FakeMeta::default();
    broken.broken.set(true);
    assert!(
        !sync_due(&broken, &clock, true),
        "no network when meta is down"
    );
}
