//! Snapshots of the merged `settings.json` for the interesting fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use serde_json::Value;
use tokred_install::settings::{hook_entries, merge_settings};

fn merged(fixture: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(fixture);
    let original: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let value = merge_settings(original, &hook_entries("/opt/homebrew/bin/tokred")).unwrap();
    serde_json::to_string_pretty(&value).unwrap()
}

#[test]
fn empty_settings() {
    insta::assert_snapshot!(merged("settings-empty.json"));
}

#[test]
fn settings_with_rtk_hooks() {
    insta::assert_snapshot!(merged("settings-rtk.json"));
}

#[test]
fn settings_with_user_hooks_in_the_same_events() {
    insta::assert_snapshot!(merged("settings-user-hooks.json"));
}

#[test]
fn settings_with_an_old_tokred_version() {
    insta::assert_snapshot!(merged("settings-old-tokred.json"));
}

#[test]
fn settings_with_unknown_keys() {
    insta::assert_snapshot!(merged("settings-unknown-keys.json"));
}
