//! Property tests of the settings merge.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use serde_json::{Map, Value, json};
use tokred_install::settings::{hook_entries, merge_settings, remove_tokred};

const EVENTS: [&str; 5] = [
    "PreToolUse",
    "Stop",
    "SessionStart",
    "UserPromptSubmit",
    "PostToolUse",
];
const FOREIGN: [&str; 4] = [
    "rtk hook claude",
    "echo done",
    "bash ./detect.sh",
    "python3 audit.py",
];
const OLD_TOKRED: [&str; 2] = [
    "\"/old/tokred\" hook guard-read",
    "\"C:\\Program Files\\tokred\\tokred.exe\" hook route",
];

fn handler(commands: &'static [&'static str]) -> impl Strategy<Value = Value> {
    (prop::sample::select(commands), prop::option::of(1u64..60)).prop_map(|(command, timeout)| {
        let mut h = json!({ "type": "command", "command": command });
        if let Some(timeout) = timeout {
            h["timeout"] = json!(timeout);
        }
        h
    })
}

fn group(commands: &'static [&'static str]) -> impl Strategy<Value = Value> {
    (
        prop::option::of(prop::sample::select(
            &["Bash", "Read", "Grep|Glob", "Edit|Write"][..],
        )),
        prop::collection::vec(handler(commands), 1..3),
    )
        .prop_map(|(matcher, handlers)| {
            let mut g = Map::new();
            if let Some(matcher) = matcher {
                g.insert("matcher".to_owned(), json!(matcher));
            }
            g.insert("hooks".to_owned(), Value::Array(handlers));
            Value::Object(g)
        })
}

/// Settings made of foreign hooks (non-empty containers only) plus unrelated keys.
fn settings(commands: &'static [&'static str]) -> impl Strategy<Value = Value> {
    let hooks = prop::collection::btree_map(
        prop::sample::select(&EVENTS[..]),
        prop::collection::vec(group(commands), 1..3),
        0..4,
    );
    (hooks, prop::option::of("[a-z]{1,8}"), any::<bool>()).prop_map(|(hooks, model, with_hooks)| {
        let mut root = Map::new();
        if let Some(model) = model {
            root.insert("model".to_owned(), json!(model));
        }
        if with_hooks && !hooks.is_empty() {
            root.insert(
                "hooks".to_owned(),
                Value::Object(
                    hooks
                        .into_iter()
                        .map(|(k, v)| (k.to_owned(), Value::Array(v)))
                        .collect(),
                ),
            );
        }
        root.insert("env".to_owned(), json!({ "A": "1" }));
        Value::Object(root)
    })
}

proptest! {
    /// configure twice = configure once, even when old tokred entries are present.
    #[test]
    fn configure_is_idempotent(
        base in settings(&FOREIGN),
        extra in prop::collection::vec(group(&OLD_TOKRED), 0..2),
    ) {
        let mut input = base;
        if !extra.is_empty() {
            input["hooks"]["PreToolUse"] = Value::Array(extra);
        }
        let entries = hook_entries("/opt/bin/tokred");
        let once = merge_settings(input, &entries).unwrap();
        let twice = merge_settings(once.clone(), &entries).unwrap();
        prop_assert_eq!(once, twice);
    }

    /// configure + unconfigure = original, for settings without tokred entries.
    #[test]
    fn configure_then_unconfigure_is_the_identity(input in settings(&FOREIGN)) {
        let entries = hook_entries("/opt/bin/tokred");
        let merged = merge_settings(input.clone(), &entries).unwrap();
        prop_assert_eq!(remove_tokred(merged), input);
    }

    /// Foreign handlers are never lost or reordered by configure.
    #[test]
    fn foreign_handlers_survive(input in settings(&FOREIGN)) {
        use tokred_install::settings::foreign_entries;
        let entries = hook_entries("/opt/bin/tokred");
        let merged = merge_settings(input.clone(), &entries).unwrap();
        prop_assert_eq!(foreign_entries(&merged), foreign_entries(&input));
    }
}
