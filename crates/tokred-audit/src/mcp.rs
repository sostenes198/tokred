//! MCP servers: reading the configuration JSON and comparing it with real usage.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde_json::Value;

use crate::ports::UsageScan;
use crate::report::{McpScope, McpServerReport, UnconfiguredUsage};

/// A server definition found in one configuration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerDef {
    pub name: String,
    pub scope: McpScope,
    pub source: PathBuf,
    pub transport: String,
    pub disabled: bool,
}

/// The name Claude Code gives a server inside tool names: every character outside
/// `[A-Za-z0-9_-]` becomes `_` (observed in transcripts such as `mcp__Claude_Browser__*`).
pub(crate) fn normalize_server_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Project paths are compared with `/` separators and no trailing separator.
pub(crate) fn project_key(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_owned()
}

/// `(name, transport)` of every entry of an `mcpServers` object (empty when it is not an object).
pub(crate) fn parse_server_map(servers: Option<&Value>) -> Vec<(String, String)> {
    let Some(map) = servers.and_then(Value::as_object) else {
        return Vec::new();
    };
    map.iter()
        .map(|(name, def)| {
            let transport = def
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("stdio")
                .to_owned();
            (name.clone(), transport)
        })
        .collect()
}

/// The `mcpServers` object of the local-scope entry of `project` in `~/.claude.json`.
pub(crate) fn local_servers<'a>(global: &'a Value, project: &str) -> Option<&'a Value> {
    let wanted = project_key(project);
    global
        .get("projects")?
        .as_object()?
        .iter()
        .find(|(key, _)| project_key(key) == wanted)?
        .1
        .get("mcpServers")
}

/// Names listed in `disabledMcpjsonServers` of a settings file.
pub(crate) fn disabled_names(settings: &Value) -> Vec<String> {
    settings
        .get("disabledMcpjsonServers")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(|n| n.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn action_for(unused: bool, disabled: bool, window_days: u32) -> Option<String> {
    match (unused, disabled) {
        (true, true) => Some(format!(
            "disabled and no calls in the last {window_days} days: remove it from the config"
        )),
        (true, false) => Some(format!(
            "no calls in the last {window_days} days: consider removing"
        )),
        _ => None,
    }
}

/// Compares configured servers with usage. `usage` is `None` when it is unknown, in which case no
/// server is reported as unused.
pub(crate) fn compare(
    defs: Vec<ServerDef>,
    usage: Option<&UsageScan>,
    window_days: u32,
) -> (Vec<McpServerReport>, Vec<UnconfiguredUsage>) {
    let mut by_name: BTreeMap<String, Vec<ServerDef>> = BTreeMap::new();
    for def in defs {
        by_name.entry(def.name.clone()).or_default().push(def);
    }
    let mut known_keys = BTreeSet::new();
    let mut reports = Vec::new();
    for (name, mut defs) in by_name {
        defs.sort_by_key(|d| d.scope);
        let key = normalize_server_name(&name);
        let used = usage.and_then(|scan| scan.servers.get(&key));
        let calls = used.map_or(0, |u| u.calls);
        let unused = usage.is_some() && calls == 0;
        let winner = defs.remove(0);
        reports.push(McpServerReport {
            action: action_for(unused, winner.disabled, window_days),
            also_in: defs.iter().map(|d| d.scope).collect(),
            source: winner.source,
            transport: winner.transport,
            disabled: winner.disabled,
            scope: winner.scope,
            last_seen_unix: used.and_then(|u| u.last_seen).map(|t| t.0),
            name,
            calls,
            unused,
        });
        known_keys.insert(key);
    }
    reports.sort_by(|a, b| {
        b.unused
            .cmp(&a.unused)
            .then(a.calls.cmp(&b.calls))
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut unconfigured: Vec<UnconfiguredUsage> = usage
        .map(|scan| {
            scan.servers
                .iter()
                .filter(|(key, _)| !known_keys.contains(*key))
                .map(|(key, u)| UnconfiguredUsage {
                    server: key.clone(),
                    calls: u.calls,
                    last_seen_unix: u.last_seen.map(|t| t.0),
                })
                .collect()
        })
        .unwrap_or_default();
    unconfigured.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.server.cmp(&b.server)));
    (reports, unconfigured)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;
    use tokred_core::Timestamp;

    use super::*;
    use crate::ports::McpUsage;

    fn def(name: &str, scope: McpScope, disabled: bool) -> ServerDef {
        ServerDef {
            name: name.to_owned(),
            scope,
            source: PathBuf::from(format!("/{scope:?}.json")),
            transport: "stdio".to_owned(),
            disabled,
        }
    }

    fn scan(entries: &[(&str, u64, Option<i64>)]) -> UsageScan {
        UsageScan {
            servers: entries
                .iter()
                .map(|(name, calls, last)| {
                    (
                        (*name).to_owned(),
                        McpUsage {
                            calls: *calls,
                            last_seen: last.map(Timestamp),
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>(),
            files_scanned: 1,
        }
    }

    #[test]
    fn names_are_normalized_like_tool_names() {
        assert_eq!(normalize_server_name("Claude Browser"), "Claude_Browser");
        assert_eq!(normalize_server_name("my-server_1"), "my-server_1");
        assert_eq!(normalize_server_name("a.b/c"), "a_b_c");
    }

    #[test]
    fn project_keys_ignore_separator_style() {
        assert_eq!(project_key("C:\\work\\app\\"), "C:/work/app");
        assert_eq!(project_key("/work/app/"), "/work/app");
    }

    #[test]
    fn parses_server_maps_defensively() {
        let value = json!({"a": {"type": "http", "url": "u"}, "b": {"command": "c"}, "c": 1});
        assert_eq!(
            parse_server_map(Some(&value)),
            [
                ("a".to_owned(), "http".to_owned()),
                ("b".to_owned(), "stdio".to_owned()),
                ("c".to_owned(), "stdio".to_owned())
            ]
        );
        assert!(parse_server_map(Some(&json!([1]))).is_empty());
        assert!(parse_server_map(None).is_empty());
    }

    #[test]
    fn finds_local_scope_by_project_path() {
        let global = json!({"projects": {"/w/app": {"mcpServers": {"x": {}}}, "/w/other": {}}});
        assert!(local_servers(&global, "/w/app/").is_some());
        assert!(local_servers(&global, "/w/other").is_none());
        assert!(local_servers(&global, "/w/none").is_none());
        assert!(local_servers(&json!({}), "/w/app").is_none());
        assert!(local_servers(&json!({"projects": 1}), "/w/app").is_none());
    }

    #[test]
    fn reads_disabled_names() {
        assert_eq!(
            disabled_names(&json!({"disabledMcpjsonServers": ["a", 1, "b"]})),
            ["a", "b"]
        );
        assert!(disabled_names(&json!({})).is_empty());
    }

    #[test]
    fn scope_precedence_is_local_then_project_then_user() {
        let defs = vec![
            def("s", McpScope::User, false),
            def("s", McpScope::Local, false),
            def("s", McpScope::Project, false),
        ];
        let (reports, _) = compare(defs, Some(&scan(&[("s", 3, Some(10))])), 30);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].scope, McpScope::Local);
        assert_eq!(reports[0].also_in, [McpScope::Project, McpScope::User]);
        assert_eq!(reports[0].calls, 3);
        assert_eq!(reports[0].last_seen_unix, Some(10));
        assert!(!reports[0].unused);
        assert_eq!(reports[0].action, None);
    }

    #[test]
    fn orders_unused_first_then_by_calls_and_reports_foreign_usage() {
        let defs = vec![
            def("zeta", McpScope::User, false),
            def("alpha", McpScope::User, true),
            def("busy", McpScope::User, false),
            def("Two Words", McpScope::User, false),
        ];
        let usage = scan(&[
            ("busy", 9, None),
            ("Two_Words", 1, Some(5)),
            ("plug", 4, None),
            ("aaa", 4, None),
        ]);
        let (reports, extra) = compare(defs, Some(&usage), 7);
        let order: Vec<&str> = reports.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(order, ["alpha", "zeta", "Two Words", "busy"]);
        assert!(reports[0].unused && reports[0].disabled);
        assert!(reports[0].action.as_deref().unwrap().contains("remove it"));
        assert!(reports[1].action.as_deref().unwrap().contains("7 days"));
        let foreign: Vec<(&str, u64)> =
            extra.iter().map(|e| (e.server.as_str(), e.calls)).collect();
        assert_eq!(foreign, [("aaa", 4), ("plug", 4)]);
    }

    #[test]
    fn unknown_usage_marks_nothing_unused() {
        let (reports, extra) = compare(vec![def("s", McpScope::User, false)], None, 30);
        assert!(!reports[0].unused);
        assert_eq!(reports[0].calls, 0);
        assert!(extra.is_empty());
    }
}
