//! Non-destructive registration of the `tokred mcp` server (the `tokred_retrieve` tool of
//! reversible compression) in Claude Code's user-scope configuration.
//!
//! Claude Code keeps user-scope MCP servers in the top-level `mcpServers` object of
//! `~/.claude.json` (or `<CLAUDE_CONFIG_DIR>/.claude.json`), next to many keys that are none of
//! tokred's business. A stdio server is `{"type": "stdio", "command": ..., "args": [...],
//! "env": {...}}` (an entry without `type` is also read as stdio).
//!
//! All functions here are pure: they take and return [`serde_json::Value`]s (with
//! `preserve_order`, so key order survives). Only the `tokred` entry is ever touched; every other
//! server and key is kept, in its original position.

use serde_json::{Map, Value, json};

/// Name of the server entry under `mcpServers`.
pub const SERVER_NAME: &str = "tokred";

/// Key of the user-scope servers object.
pub const SERVERS_KEY: &str = "mcpServers";

/// Arguments that start the MCP server.
pub const SERVER_ARGS: [&str; 1] = ["mcp"];

/// The configuration cannot be merged safely: nothing should be written.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum McpMergeError {
    /// The document root is not a JSON object.
    #[error("the Claude Code configuration root must be a JSON object")]
    RootNotObject,
    /// `mcpServers` exists but is not an object.
    #[error("`mcpServers` must be a JSON object")]
    ServersNotObject,
}

/// The entry tokred registers for the binary at `binary` (its stable path).
#[must_use]
pub fn server_entry(binary: &str) -> Value {
    json!({ "type": "stdio", "command": binary, "args": SERVER_ARGS, "env": {} })
}

/// Adds the `tokred` server, or updates it in place (same position) when it exists. Of an
/// existing object entry only `type`, `command` and `args` are set, so keys the user added (such
/// as `env`) stay. Registering twice gives the same result as registering once.
pub fn register(mut config: Value, binary: &str) -> Result<Value, McpMergeError> {
    let root = config.as_object_mut().ok_or(McpMergeError::RootNotObject)?;
    let servers = root
        .entry(SERVERS_KEY)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or(McpMergeError::ServersNotObject)?;
    match servers.get_mut(SERVER_NAME).and_then(Value::as_object_mut) {
        Some(entry) => {
            entry.insert("type".to_owned(), json!("stdio"));
            entry.insert("command".to_owned(), json!(binary));
            entry.insert("args".to_owned(), json!(SERVER_ARGS));
        }
        None => {
            servers.insert(SERVER_NAME.to_owned(), server_entry(binary));
        }
    }
    Ok(config)
}

/// Removes the `tokred` server. `mcpServers` is dropped only when this removal is what emptied
/// it. Anything it does not understand is left as it is. Never fails.
#[must_use]
pub fn unregister(mut config: Value) -> Value {
    let Some(root) = config.as_object_mut() else {
        return config;
    };
    let emptied = root
        .get_mut(SERVERS_KEY)
        .and_then(Value::as_object_mut)
        .is_some_and(|servers| servers.shift_remove(SERVER_NAME).is_some() && servers.is_empty());
    if emptied {
        root.shift_remove(SERVERS_KEY);
    }
    config
}

/// The `command` of the registered `tokred` stdio server, if there is one.
#[must_use]
pub fn registered_command(config: &Value) -> Option<&str> {
    let entry = config.get(SERVERS_KEY)?.get(SERVER_NAME)?;
    let stdio = entry.get("type").is_none_or(|t| t == "stdio");
    let args_ok = entry.get("args") == Some(&json!(SERVER_ARGS));
    entry
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| stdio && args_ok && !command.is_empty())
}

/// Whether the `tokred` stdio server is registered (with `args = ["mcp"]`). The compression
/// hook only enforces when this is true.
#[must_use]
pub fn is_registered(config: &Value) -> bool {
    registered_command(config).is_some()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const BIN: &str = "/opt/homebrew/bin/tokred";

    /// A `~/.claude.json` shaped like the real one: unrelated keys around `mcpServers`.
    fn fixture() -> Value {
        json!({
            "numStartups": 42,
            "theme": "dark",
            "mcpServers": {
                "bigquery": {"type": "stdio", "command": "npx", "args": ["-y", "bq"],
                             "env": {"KEY": "value"}},
                "docs": {"type": "http", "url": "https://example.com/mcp"}
            },
            "projects": {"/repo": {"mcpServers": {"local": {"command": "x"}}}},
            "userID": "abc"
        })
    }

    #[test]
    fn registering_adds_the_entry_after_the_others() {
        let merged = register(fixture(), BIN).unwrap();
        let servers = merged[SERVERS_KEY].as_object().unwrap();
        assert_eq!(
            servers.keys().collect::<Vec<_>>(),
            ["bigquery", "docs", "tokred"]
        );
        assert_eq!(servers["tokred"], server_entry(BIN));
        assert_eq!(
            servers["tokred"],
            json!({"type": "stdio", "command": BIN, "args": ["mcp"], "env": {}})
        );
        let keys: Vec<&String> = merged.as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["numStartups", "theme", "mcpServers", "projects", "userID"]
        );
        assert_eq!(
            merged["projects"],
            fixture()["projects"],
            "local scope untouched"
        );
        assert!(is_registered(&merged));
        assert_eq!(registered_command(&merged), Some(BIN));
        assert!(!is_registered(&fixture()));
    }

    #[test]
    fn registering_creates_mcp_servers_when_missing() {
        let merged = register(json!({"theme": "dark"}), BIN).unwrap();
        assert_eq!(
            merged,
            json!({"theme": "dark", "mcpServers": {"tokred": server_entry(BIN)}})
        );
        assert_eq!(unregister(merged), json!({"theme": "dark"}));
    }

    #[test]
    fn an_existing_entry_is_updated_in_place_keeping_user_keys() {
        let config = json!({"mcpServers": {
            "a": {"command": "a"},
            "tokred": {"command": "/old/tokred", "args": ["mcp"], "env": {"RUST_LOG": "debug"}},
            "z": {"command": "z"}
        }});
        let merged = register(config, BIN).unwrap();
        let servers = merged[SERVERS_KEY].as_object().unwrap();
        assert_eq!(servers.keys().collect::<Vec<_>>(), ["a", "tokred", "z"]);
        assert_eq!(
            servers["tokred"],
            json!({"command": BIN, "args": ["mcp"], "env": {"RUST_LOG": "debug"}, "type": "stdio"})
        );

        // An entry that is not even an object is replaced, still in place.
        let merged = register(json!({"mcpServers": {"tokred": "junk", "z": {}}}), BIN).unwrap();
        assert_eq!(
            merged[SERVERS_KEY]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            ["tokred", "z"]
        );
        assert!(is_registered(&merged));
    }

    #[test]
    fn unregistering_removes_only_tokred_and_keeps_the_order() {
        let config = json!({"mcpServers": {"a": {}, "tokred": server_entry(BIN), "b": {}}, "x": 1});
        let removed = unregister(config);
        assert_eq!(removed, json!({"mcpServers": {"a": {}, "b": {}}, "x": 1}));
        assert_eq!(
            removed[SERVERS_KEY]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        // Nothing to remove: unchanged, including an empty `mcpServers` that was already there.
        assert_eq!(unregister(fixture()), fixture());
        assert_eq!(
            unregister(json!({"mcpServers": {}})),
            json!({"mcpServers": {}})
        );
        assert_eq!(
            unregister(json!({"mcpServers": []})),
            json!({"mcpServers": []})
        );
        assert_eq!(unregister(json!([1])), json!([1]));
        // The documented exception: an empty `mcpServers` that tokred filled is dropped again.
        let filled = register(json!({"mcpServers": {}}), BIN).unwrap();
        assert_eq!(unregister(filled), json!({}));
    }

    #[test]
    fn shapes_that_cannot_be_merged_are_refused() {
        assert_eq!(register(json!([]), BIN), Err(McpMergeError::RootNotObject));
        assert_eq!(
            register(json!({"mcpServers": "no"}), BIN),
            Err(McpMergeError::ServersNotObject)
        );
        assert!(
            McpMergeError::ServersNotObject
                .to_string()
                .contains("mcpServers")
        );
        assert!(McpMergeError::RootNotObject.to_string().contains("root"));
    }

    #[test]
    fn only_a_complete_stdio_entry_counts_as_registered() {
        for entry in [
            json!({"type": "http", "url": "https://x", "command": BIN, "args": ["mcp"]}),
            json!({"command": BIN, "args": ["serve"]}),
            json!({"command": BIN}),
            json!({"command": "", "args": ["mcp"]}),
            json!({"args": ["mcp"]}),
        ] {
            let config = json!({"mcpServers": {"tokred": entry}});
            assert!(!is_registered(&config), "{config}");
        }
        assert!(is_registered(
            &json!({"mcpServers": {"tokred": {"command": BIN, "args": ["mcp"]}}})
        ));
        assert!(!is_registered(&json!({})));
        assert!(!is_registered(&json!({"mcpServers": {"other": {}}})));
    }

    fn other_value() -> impl Strategy<Value = Value> {
        prop_oneof![
            any::<i64>().prop_map(Value::from),
            "[a-z ]{0,8}".prop_map(Value::from),
            Just(json!({"command": "npx", "args": ["-y", "x"]})),
        ]
    }

    /// A configuration without tokred: unrelated keys and an optional, non-empty `mcpServers`.
    fn config_without_tokred() -> impl Strategy<Value = Value> {
        let keys = prop::collection::vec(("[a-zA-Z]{1,8}", other_value()), 0..6);
        let servers = prop::option::of(prop::collection::vec(
            (
                "[a-z]{1,6}".prop_filter("not tokred", |n| n != SERVER_NAME),
                other_value(),
            ),
            1..4,
        ));
        (keys, servers, 0usize..7).prop_map(|(keys, servers, position)| {
            let mut root = Map::new();
            for (key, value) in keys.into_iter().filter(|(k, _)| k != SERVERS_KEY) {
                root.insert(key, value);
            }
            if let Some(servers) = servers {
                let servers: Map<String, Value> = servers.into_iter().collect();
                let at = position.min(root.len());
                root.shift_insert(at, SERVERS_KEY.to_owned(), Value::Object(servers));
            }
            Value::Object(root)
        })
    }

    proptest! {
        #[test]
        fn register_is_idempotent_and_reversible(config in config_without_tokred()) {
            let once = register(config.clone(), BIN).unwrap();
            let twice = register(once.clone(), BIN).unwrap();
            prop_assert_eq!(&twice, &once);
            prop_assert_eq!(twice.to_string(), once.to_string());
            prop_assert!(is_registered(&once));

            let removed = unregister(once);
            prop_assert_eq!(&removed, &config);
            prop_assert_eq!(removed.to_string(), config.to_string(), "same key order");
            prop_assert_eq!(unregister(removed), config);
        }
    }
}
