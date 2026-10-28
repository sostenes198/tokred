//! Non-destructive merge of tokred hooks into Claude Code's `settings.json`.
//!
//! All functions here are pure: they take and return [`serde_json::Value`]s. Only entries whose
//! `command` invokes `tokred hook <name>` (see [`parse_tokred_command`]) are ever touched; every
//! other key, hook and group is kept, in its original order.

use serde::Serialize;
use serde_json::{Map, Value, json};
use tokred_core::HookEvent;

use crate::error::MergeError;

/// Timeout, in seconds, of every tokred hook (Claude Code's `timeout` unit is seconds).
pub const HOOK_TIMEOUT_SECS: u64 = 5;

/// One hook handler that tokred wants in `settings.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookEntry {
    /// Claude Code event name (`PreToolUse`, `Stop`, ...).
    pub event: String,
    /// Tool matcher of the group, if any.
    pub matcher: Option<String>,
    /// Full command line.
    pub command: String,
    /// Timeout in seconds.
    pub timeout: u64,
}

impl HookEntry {
    /// The `<arg>` of `tokred hook <arg>`, when the command is a tokred one.
    #[must_use]
    pub fn hook_arg(&self) -> Option<&str> {
        hook_arg(&self.command)
    }
}

fn hook_arg(command: &str) -> Option<&str> {
    parse_tokred_command(command)
}

const HOOK_WORD: &str = " hook ";

/// Recognizes a tokred hook command and returns its `<name>`.
///
/// A command is tokred's when it runs an executable named `tokred` (or `tokred.exe`), optionally
/// quoted, followed by `hook <name>`. The spec describes this as "contains `tokred hook `", but
/// its own example (`"/path/tokred" hook guard-read`) has a closing quote in between, so a plain
/// substring match would never find the entries tokred itself writes.
#[must_use]
pub fn parse_tokred_command(command: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(offset) = command[from..].find(HOOK_WORD) {
        let at = from + offset;
        let executable = command[..at].trim_end_matches('"');
        let lower = executable.to_ascii_lowercase();
        let stem = lower
            .strip_suffix("tokred.exe")
            .or_else(|| lower.strip_suffix("tokred"));
        let boundary_ok = stem.is_some_and(|stem| {
            stem.chars()
                .next_back()
                .is_none_or(|c| matches!(c, '/' | '\\' | '"' | ' '))
        });
        if boundary_ok {
            return command[at + HOOK_WORD.len()..].split_whitespace().next();
        }
        from = at + 1;
    }
    None
}

/// The hook entries tokred installs, for the binary found at `binary`.
///
/// The path is always quoted because on Windows it may contain spaces.
#[must_use]
pub fn hook_entries(binary: &str) -> Vec<HookEntry> {
    HookEntry::all(binary)
}

impl HookEntry {
    fn all(binary: &str) -> Vec<Self> {
        HookEvent::ALL
            .into_iter()
            .map(|hook| Self {
                event: hook.claude_event().to_owned(),
                matcher: hook.matcher().map(str::to_owned),
                command: format!("\"{binary}\"{HOOK_WORD}{}", hook.arg()),
                timeout: HOOK_TIMEOUT_SECS,
            })
            .collect()
    }
}

fn is_tokred_command(handler: &Value) -> bool {
    handler
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|c| hook_arg(c).is_some())
}

fn hooks_object_mut(root: &mut Value) -> Result<Option<&mut Map<String, Value>>, MergeError> {
    let object = root.as_object_mut().ok_or(MergeError::RootNotObject)?;
    match object.get_mut("hooks") {
        None => Ok(None),
        Some(hooks) => hooks
            .as_object_mut()
            .map(Some)
            .ok_or(MergeError::HooksNotObject),
    }
}

/// Removes every tokred handler. Groups, events and the `hooks` key are dropped only when this
/// removal is what emptied them. Returns how many handlers were removed.
fn strip(root: &mut Value) -> Result<usize, MergeError> {
    let mut removed_total = 0;
    let mut emptied_events: Vec<String> = Vec::new();
    let hooks_now_empty;
    {
        let Some(hooks) = hooks_object_mut(root)? else {
            return Ok(0);
        };
        for (event, groups) in hooks.iter_mut() {
            let groups = groups
                .as_array_mut()
                .ok_or_else(|| MergeError::EventNotArray(event.clone()))?;
            let mut removed_in_event = 0;
            let mut kept = Vec::with_capacity(groups.len());
            for mut group in std::mem::take(groups) {
                let removed = strip_group(&mut group);
                removed_in_event += removed;
                let emptied = removed > 0 && group_handlers_empty(&group);
                if !emptied {
                    kept.push(group);
                }
            }
            *groups = kept;
            if removed_in_event > 0 && groups.is_empty() {
                emptied_events.push(event.clone());
            }
            removed_total += removed_in_event;
        }
        for event in &emptied_events {
            hooks.shift_remove(event);
        }
        hooks_now_empty = hooks.is_empty();
    }
    if let Some(object) = root
        .as_object_mut()
        .filter(|_| removed_total > 0 && hooks_now_empty)
    {
        object.shift_remove("hooks");
    }
    Ok(removed_total)
}

fn strip_group(group: &mut Value) -> usize {
    let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
        return 0;
    };
    let before = handlers.len();
    handlers.retain(|handler| !is_tokred_command(handler));
    before - handlers.len()
}

fn group_handlers_empty(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
}

/// Removes tokred's handlers (the `unconfigure` algorithm).
pub fn remove_tokred(mut current: Value) -> Result<Value, MergeError> {
    strip(&mut current)?;
    Ok(current)
}

/// Installs `entries`, replacing any previous tokred handlers (the `configure` algorithm).
///
/// Each entry goes in a group of its own, appended after existing groups, so tokred never shares
/// a group with the user's or rtk's hooks. Running it twice gives the same result as running it
/// once.
pub fn merge_settings(mut current: Value, entries: &[HookEntry]) -> Result<Value, MergeError> {
    strip(&mut current)?;
    let object = current.as_object_mut().ok_or(MergeError::RootNotObject)?;
    if entries.is_empty() {
        return Ok(current);
    }
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or(MergeError::HooksNotObject)?;
    for entry in entries {
        let groups = hooks
            .entry(entry.event.clone())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| MergeError::EventNotArray(entry.event.clone()))?;
        let handler =
            json!({ "type": "command", "command": entry.command, "timeout": entry.timeout });
        let mut group = Map::new();
        if let Some(matcher) = &entry.matcher {
            group.insert("matcher".to_owned(), Value::String(matcher.clone()));
        }
        group.insert("hooks".to_owned(), Value::Array(vec![handler]));
        groups.push(Value::Object(group));
    }
    Ok(current)
}

/// Lists the tokred handlers currently present in `settings`.
#[must_use]
pub fn tokred_entries(settings: &Value) -> Vec<HookEntry> {
    collect_handlers(settings, |command| hook_arg(command).is_some())
}

/// Lists the handlers that are **not** tokred's (the user's, rtk's, ...).
#[must_use]
pub fn foreign_entries(settings: &Value) -> Vec<HookEntry> {
    collect_handlers(settings, |command| hook_arg(command).is_none())
}

fn collect_handlers(settings: &Value, keep: impl Fn(&str) -> bool) -> Vec<HookEntry> {
    let mut out = Vec::new();
    let Some(hooks) = settings.get("hooks").and_then(Value::as_object) else {
        return out;
    };
    for (event, groups) in hooks {
        for group in groups.as_array().into_iter().flatten() {
            let matcher = group
                .get("matcher")
                .and_then(Value::as_str)
                .map(str::to_owned);
            for handler in group
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(command) = handler.get("command").and_then(Value::as_str) else {
                    continue;
                };
                if keep(command) {
                    out.push(HookEntry {
                        event: event.clone(),
                        matcher: matcher.clone(),
                        command: command.to_owned(),
                        timeout: handler.get("timeout").and_then(Value::as_u64).unwrap_or(0),
                    });
                }
            }
        }
    }
    out
}

/// What `configure` will do to one hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    /// The hook is not installed yet.
    Added,
    /// A tokred hook with the same name exists but differs (path, matcher or timeout).
    Updated,
    /// Already up to date.
    Unchanged,
    /// A tokred hook that this version no longer installs.
    Removed,
}

/// Planned change for one hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookChange {
    /// The hook name (`guard-read`, ...) or the command when it has no recognizable name.
    pub hook: String,
    /// Claude Code event.
    pub event: String,
    /// What will happen.
    pub kind: ChangeKind,
}

/// Compares the tokred handlers in `current` with the `desired` ones.
#[must_use]
pub fn plan_hook_changes(current: &Value, desired: &[HookEntry]) -> Vec<HookChange> {
    let existing = tokred_entries(current);
    let name = |entry: &HookEntry| entry.hook_arg().unwrap_or(&entry.command).to_owned();
    let mut changes: Vec<HookChange> = desired
        .iter()
        .map(|want| {
            let same_name = existing.iter().filter(|have| name(have) == name(want));
            let mut same_name = same_name.peekable();
            let kind = if same_name.peek().is_none() {
                ChangeKind::Added
            } else if same_name.any(|have| have == want) {
                ChangeKind::Unchanged
            } else {
                ChangeKind::Updated
            };
            HookChange {
                hook: name(want),
                event: want.event.clone(),
                kind,
            }
        })
        .collect();
    for have in &existing {
        if !desired.iter().any(|want| name(want) == name(have)) {
            changes.push(HookChange {
                hook: name(have),
                event: have.event.clone(),
                kind: ChangeKind::Removed,
            });
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<HookEntry> {
        hook_entries("/opt/bin/tokred")
    }

    #[test]
    fn commands_quote_the_binary_and_are_recognized_as_tokred_hooks() {
        for entry in entries() {
            assert!(
                entry.command.starts_with("\"/opt/bin/tokred\" hook "),
                "{}",
                entry.command
            );
            assert!(entry.hook_arg().is_some(), "{}", entry.command);
        }
        assert_eq!(entries()[0].hook_arg(), Some("route"));
    }

    #[test]
    fn parse_recognizes_tokred_commands_only() {
        let cases = [
            (
                "\"/opt/homebrew/bin/tokred\" hook guard-read",
                Some("guard-read"),
            ),
            ("tokred hook route", Some("route")),
            (
                "\"C:\\Program Files\\tokred\\tokred.exe\" hook stop",
                Some("stop"),
            ),
            ("rtk hook claude", None),
            ("my-tokred hook x", None),
            ("echo tokred", None),
            ("bash -c 'tokred hook a'", None),
        ];
        for (command, expected) in cases {
            assert_eq!(parse_tokred_command(command), expected, "{command}");
        }
    }

    #[test]
    fn merge_into_empty_object() {
        let merged = merge_settings(json!({}), &entries()).unwrap();
        assert_eq!(tokred_entries(&merged), entries());
        assert_eq!(merged["hooks"]["PreToolUse"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn merge_keeps_foreign_hooks_in_their_own_groups() {
        let current = json!({
            "hooks": { "PreToolUse": [
                { "matcher": "Bash", "hooks": [{ "type": "command", "command": "rtk hook claude" }] }
            ]}
        });
        let merged = merge_settings(current, &entries()).unwrap();
        let groups = merged["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(groups[0]["hooks"][0]["command"], "rtk hook claude");
        assert_eq!(groups.len(), 4);
        assert_eq!(foreign_entries(&merged).len(), 1);
    }

    #[test]
    fn merge_twice_equals_once() {
        let once = merge_settings(json!({"model": "opus"}), &entries()).unwrap();
        let twice = merge_settings(once.clone(), &entries()).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn remove_restores_original_and_drops_emptied_containers() {
        let original = json!({ "model": "opus", "env": { "A": "1" } });
        let merged = merge_settings(original.clone(), &entries()).unwrap();
        assert_eq!(remove_tokred(merged).unwrap(), original);
    }

    #[test]
    fn remove_keeps_containers_that_still_hold_foreign_entries() {
        let original = json!({ "hooks": { "Stop": [
            { "hooks": [{ "type": "command", "command": "echo done" }] }
        ]}});
        let merged = merge_settings(original.clone(), &entries()).unwrap();
        assert_eq!(remove_tokred(merged).unwrap(), original);
    }

    #[test]
    fn remove_does_not_touch_containers_that_were_already_empty() {
        let original = json!({ "hooks": { "Notification": [] } });
        assert_eq!(remove_tokred(original.clone()).unwrap(), original);
    }

    #[test]
    fn old_tokred_entries_are_replaced() {
        let old = json!({ "hooks": { "PreToolUse": [
            { "matcher": "Read", "hooks": [
                { "type": "command", "command": "\"/old/tokred\" hook guard-read", "timeout": 9 }
            ]}
        ]}});
        let merged = merge_settings(old, &entries()).unwrap();
        let read_hooks: Vec<_> = tokred_entries(&merged)
            .into_iter()
            .filter(|e| e.hook_arg() == Some("guard-read"))
            .collect();
        assert_eq!(read_hooks.len(), 1);
        assert!(read_hooks[0].command.contains("/opt/bin/tokred"));
    }

    #[test]
    fn mixed_group_keeps_the_foreign_handler() {
        let current = json!({ "hooks": { "Stop": [{ "hooks": [
            { "type": "command", "command": "echo hi" },
            { "type": "command", "command": "\"/x/tokred\" hook session-check" }
        ]}]}});
        let stripped = remove_tokred(current).unwrap();
        assert_eq!(
            stripped,
            json!({ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "command": "echo hi" }]}]}})
        );
    }

    #[test]
    fn bad_shapes_are_errors_not_panics() {
        assert_eq!(
            merge_settings(json!([]), &entries()),
            Err(MergeError::RootNotObject)
        );
        assert_eq!(
            merge_settings(json!({"hooks": 1}), &entries()),
            Err(MergeError::HooksNotObject)
        );
        assert_eq!(
            merge_settings(json!({"hooks": {"Stop": {}}}), &entries()),
            Err(MergeError::EventNotArray("Stop".to_owned()))
        );
    }

    #[test]
    fn plan_classifies_changes() {
        let fresh = plan_hook_changes(&json!({}), &entries());
        assert!(fresh.iter().all(|c| c.kind == ChangeKind::Added));

        let installed = merge_settings(json!({}), &entries()).unwrap();
        let again = plan_hook_changes(&installed, &entries());
        assert!(again.iter().all(|c| c.kind == ChangeKind::Unchanged));

        let moved = plan_hook_changes(&installed, &hook_entries("/new/tokred"));
        assert!(moved.iter().all(|c| c.kind == ChangeKind::Updated));
    }
}
