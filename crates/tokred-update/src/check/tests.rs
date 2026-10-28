use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use tokred_core::error::StoreError;
use tokred_core::pricing::SourceError;
use tokred_core::release::Release;
use tokred_core::types::Timestamp;

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
    fail_writes: Cell<bool>,
}

impl FakeMeta {
    fn get(&self, key: &str) -> Option<String> {
        self.map.borrow().get(key).cloned()
    }

    fn put(&self, key: &str, value: &str) {
        self.map
            .borrow_mut()
            .insert(key.to_owned(), value.to_owned());
    }
}

impl MetaRepo for FakeMeta {
    fn meta_get(&self, key: &str) -> Result<Option<String>, StoreError> {
        if self.broken.get() {
            return Err(StoreError::backend(std::io::Error::other("down")));
        }
        Ok(self.get(key))
    }

    fn meta_set(&self, key: &str, value: &str) -> Result<(), StoreError> {
        if self.broken.get() || self.fail_writes.get() {
            return Err(StoreError::backend(std::io::Error::other("down")));
        }
        self.put(key, value);
        Ok(())
    }
}

struct FakeSource {
    answer: Result<String, ()>,
    calls: Cell<u32>,
}

impl FakeSource {
    fn ok(version: &str) -> Self {
        Self {
            answer: Ok(version.to_owned()),
            calls: Cell::new(0),
        }
    }

    fn offline() -> Self {
        Self {
            answer: Err(()),
            calls: Cell::new(0),
        }
    }
}

impl ReleaseSource for FakeSource {
    fn latest(&self) -> Result<Release, SourceError> {
        self.calls.set(self.calls.get() + 1);
        match &self.answer {
            Ok(version) => Ok(Release {
                version: version.clone(),
                url: "https://example.invalid/release".to_owned(),
            }),
            Err(()) => Err(SourceError::Invalid("offline".to_owned())),
        }
    }
}

fn newer(version: &str) -> NewVersion {
    NewVersion {
        version: version.to_owned(),
    }
}

#[test]
fn mode_and_environment_decide_whether_to_check() {
    assert!(should_check(UpdateMode::Notify, false));
    assert!(should_check(UpdateMode::Auto, false));
    assert!(!should_check(UpdateMode::Off, false));
    assert!(
        !should_check(UpdateMode::Notify, true),
        "TOKRED_NO_UPDATE_CHECK"
    );
    assert!(!should_check(UpdateMode::Off, true));
}

#[test]
fn auto_mode_self_updates_script_installs_only() {
    assert!(should_self_update(UpdateMode::Auto, InstallMethod::Script));
    assert!(!should_self_update(
        UpdateMode::Notify,
        InstallMethod::Script
    ));
    assert!(!should_self_update(UpdateMode::Off, InstallMethod::Script));
    for method in [
        InstallMethod::Homebrew,
        InstallMethod::Winget,
        InstallMethod::Scoop,
        InstallMethod::Cargo,
    ] {
        assert!(!should_self_update(UpdateMode::Auto, method), "{method:?}");
    }
}

#[test]
fn the_first_run_queries_and_caches_the_answer() {
    let meta = FakeMeta::default();
    let clock = FakeClock(Cell::new(100 * DAY));
    let source = FakeSource::ok("v0.4.0");
    assert!(check_due(&meta, &clock));
    assert_eq!(
        run_check(&source, &meta, &clock, "0.3.2"),
        Some(newer("0.4.0"))
    );
    assert_eq!(source.calls.get(), 1);
    assert_eq!(meta.get(VERSION_CHECK_AT).unwrap(), (100 * DAY).to_string());
    assert_eq!(meta.get(VERSION_CHECK_LATEST).as_deref(), Some("0.4.0"));
}

#[test]
fn a_second_call_within_a_day_reuses_the_cache_without_a_request() {
    let meta = FakeMeta::default();
    let clock = FakeClock(Cell::new(100 * DAY));
    let source = FakeSource::ok("0.4.0");
    run_check(&source, &meta, &clock, "0.3.2");

    clock.0.set(100 * DAY + DAY - 1);
    assert!(!check_due(&meta, &clock));
    assert_eq!(
        run_check(&source, &meta, &clock, "0.3.2"),
        Some(newer("0.4.0"))
    );
    assert_eq!(source.calls.get(), 1, "no second request");

    clock.0.set(100 * DAY + DAY);
    assert!(check_due(&meta, &clock));
    run_check(&source, &meta, &clock, "0.3.2");
    assert_eq!(source.calls.get(), 2);
}

#[test]
fn an_upgraded_user_stops_seeing_the_notice_without_a_request() {
    let meta = FakeMeta::default();
    let clock = FakeClock(Cell::new(100 * DAY));
    let source = FakeSource::ok("0.4.0");
    assert!(run_check(&source, &meta, &clock, "0.3.2").is_some());
    assert_eq!(run_check(&source, &meta, &clock, "0.4.0"), None);
    assert_eq!(run_check(&source, &meta, &clock, "0.5.0"), None);
    assert_eq!(source.calls.get(), 1);
}

#[test]
fn network_errors_are_silent_and_not_retried_until_tomorrow() {
    let meta = FakeMeta::default();
    let clock = FakeClock(Cell::new(100 * DAY));
    let source = FakeSource::offline();
    assert_eq!(run_check(&source, &meta, &clock, "0.3.2"), None);
    assert_eq!(meta.get(VERSION_CHECK_AT).unwrap(), (100 * DAY).to_string());
    assert_eq!(meta.get(VERSION_CHECK_LATEST), None);
    assert_eq!(run_check(&source, &meta, &clock, "0.3.2"), None);
    assert_eq!(source.calls.get(), 1, "the failed attempt counts");
}

#[test]
fn a_failed_query_keeps_the_previous_answer() {
    let meta = FakeMeta::default();
    meta.put(VERSION_CHECK_AT, &(98 * DAY).to_string());
    meta.put(VERSION_CHECK_LATEST, "0.4.0");
    let clock = FakeClock(Cell::new(100 * DAY));
    let source = FakeSource::offline();
    assert_eq!(
        run_check(&source, &meta, &clock, "0.3.2"),
        Some(newer("0.4.0"))
    );
    assert_eq!(source.calls.get(), 1);
}

#[test]
fn unparsable_release_versions_are_not_cached() {
    let meta = FakeMeta::default();
    let clock = FakeClock(Cell::new(100 * DAY));
    let source = FakeSource::ok("nightly");
    assert_eq!(run_check(&source, &meta, &clock, "0.3.2"), None);
    assert_eq!(meta.get(VERSION_CHECK_LATEST), None);
}

#[test]
fn odd_stored_values_are_handled() {
    let clock = FakeClock(Cell::new(100 * DAY));
    let meta = FakeMeta::default();
    meta.put(VERSION_CHECK_AT, "yesterday");
    assert!(check_due(&meta, &clock), "unparsable counts as never");
    meta.put(VERSION_CHECK_AT, &(101 * DAY).to_string());
    assert!(check_due(&meta, &clock), "a clock that went back");

    // A cache written by hand with a leading v is normalised; garbage is ignored.
    meta.put(VERSION_CHECK_AT, &(100 * DAY).to_string());
    meta.put(VERSION_CHECK_LATEST, " v0.4.0 ");
    let source = FakeSource::offline();
    assert_eq!(
        run_check(&source, &meta, &clock, "0.3.2"),
        Some(newer("0.4.0"))
    );
    meta.put(VERSION_CHECK_LATEST, "latest");
    assert_eq!(run_check(&source, &meta, &clock, "0.3.2"), None);
    assert_eq!(source.calls.get(), 0);
}

#[test]
fn a_broken_store_means_no_request_and_no_notice() {
    let meta = FakeMeta::default();
    meta.broken.set(true);
    let clock = FakeClock(Cell::new(100 * DAY));
    let source = FakeSource::ok("0.4.0");
    assert!(!check_due(&meta, &clock));
    assert_eq!(run_check(&source, &meta, &clock, "0.3.2"), None);
    assert_eq!(source.calls.get(), 0);

    // Writes failing alone (read-only database): still answers from what it read.
    let meta = FakeMeta::default();
    meta.fail_writes.set(true);
    assert_eq!(run_check(&source, &meta, &clock, "0.3.2"), None);
    assert_eq!(source.calls.get(), 1);
}

#[test]
fn the_notice_names_the_upgrade_command() {
    let new = NewVersion {
        version: "0.4.0".to_owned(),
    };
    assert_eq!(
        notice(&new, "0.3.2", InstallMethod::Homebrew),
        "new version 0.4.0 available (current 0.3.2) \u{2192} run: brew upgrade tokred"
    );
    assert!(notice(&new, "0.3.2", InstallMethod::Winget).ends_with("winget upgrade tokred"));
    assert!(notice(&new, "0.3.2", InstallMethod::Scoop).ends_with("scoop update tokred"));
    assert!(notice(&new, "0.3.2", InstallMethod::Cargo).ends_with("cargo install tokred --force"));
    assert!(notice(&new, "0.3.2", InstallMethod::Script).ends_with("run: tokred update"));
}
