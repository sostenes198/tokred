//! Reads the parts of a `/v1/messages` body that reversible compression acts on: the text of
//! `tool_result` blocks in user messages, the tool that produced each one, and the retrievals the
//! model already made. Never looks at `system`, `tools`, `thinking`, text the model wrote or any
//! non-text content.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::preview::{PreviewRules, Replacement, content_hash, replacement};

/// Name of the retrieval tool. Claude Code shows it to the model as
/// `mcp__tokred__tokred_retrieve`, so a tool whose name ends with this is the retrieval tool.
pub const RETRIEVE_TOOL: &str = "tokred_retrieve";

/// Tool name used when a result's `tool_use_id` has no matching `tool_use` in the history.
pub const UNKNOWN_TOOL: &str = "unknown";

/// Whether `name` is the retrieval tool. Its results are never replaced: replacing a retrieval
/// with another preview would send the model in circles.
#[must_use]
pub fn is_retrieve_tool(name: &str) -> bool {
    name.ends_with(RETRIEVE_TOOL)
}

/// Where a text unit lives in the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    message: usize,
    block: usize,
    /// `None` when the `tool_result` content is a string; otherwise the index of the text block.
    item: Option<usize>,
}

impl Slot {
    /// JSON pointer to the string that holds the text.
    pub(crate) fn pointer(&self) -> String {
        let base = format!("/messages/{}/content/{}/content", self.message, self.block);
        match self.item {
            None => base,
            Some(item) => format!("{base}/{item}/text"),
        }
    }
}

/// One text a `tool_result` carries: its string content, or one of its text blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unit {
    pub(crate) slot: Slot,
    /// blake3 (hex) of the text.
    pub(crate) hash: String,
    /// The tool that produced it.
    pub(crate) tool: String,
    /// It is in the last message of the request, which is a user message: the first time the
    /// model would see it.
    pub(crate) in_last: bool,
    /// Its replacement, computed only for texts in the last message.
    pub(crate) candidate: Option<Replacement>,
}

/// What a request body holds for compression.
#[derive(Debug, Default)]
pub(crate) struct Scan {
    /// Text units of eligible tool results, in body order.
    pub(crate) units: Vec<Unit>,
    /// Retrieval id -> number of distinct retrieval calls in the history.
    pub(crate) retrievals: HashMap<String, usize>,
}

fn role(message: &Value) -> Option<&str> {
    message.get("role").and_then(Value::as_str)
}

fn blocks(message: &Value) -> &[Value] {
    message
        .get("content")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn kind(block: &Value) -> Option<&str> {
    block.get("type").and_then(Value::as_str)
}

fn is_text_item(item: &Value) -> bool {
    kind(item) == Some("text") && item.get("text").is_some_and(Value::is_string)
}

/// Reads a request body. Eligible results are those that are not errors, not retrievals, and
/// whose content is a string or an array of text blocks only (a result that mixes text with an
/// image or a document is left alone entirely).
pub(crate) fn scan(body: &Value, rules: &PreviewRules) -> Scan {
    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return Scan::default();
    };
    let mut tools: HashMap<&str, &str> = HashMap::new();
    let mut calls: HashMap<String, HashSet<&str>> = HashMap::new();
    for message in messages.iter().filter(|m| role(m) == Some("assistant")) {
        for block in blocks(message)
            .iter()
            .filter(|b| kind(b) == Some("tool_use"))
        {
            let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
            let name = block
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            tools.insert(id, name);
            if let Some(target) = block.pointer("/input/id").and_then(Value::as_str)
                && is_retrieve_tool(name)
            {
                calls
                    .entry(target.trim().to_owned())
                    .or_default()
                    .insert(id);
            }
        }
    }

    let last = messages.len().saturating_sub(1);
    let mut units = Vec::new();
    for (m, message) in messages.iter().enumerate() {
        if role(message) != Some("user") {
            continue;
        }
        let in_last = m == last;
        for (b, block) in blocks(message).iter().enumerate() {
            if kind(block) != Some("tool_result")
                || block.get("is_error").and_then(Value::as_bool) == Some(true)
            {
                continue;
            }
            let tool = block
                .get("tool_use_id")
                .and_then(Value::as_str)
                .and_then(|id| tools.get(id).copied())
                .unwrap_or(UNKNOWN_TOOL);
            if is_retrieve_tool(tool) {
                continue;
            }
            let mut push = |item: Option<usize>, text: &str| {
                units.push(Unit {
                    slot: Slot {
                        message: m,
                        block: b,
                        item,
                    },
                    hash: content_hash(text),
                    tool: tool.to_owned(),
                    in_last,
                    candidate: if in_last {
                        replacement(text, rules)
                    } else {
                        None
                    },
                });
            };
            match block.get("content") {
                Some(Value::String(text)) => push(None, text),
                Some(Value::Array(items)) if items.iter().all(is_text_item) => {
                    for (i, item) in items.iter().enumerate() {
                        push(Some(i), item["text"].as_str().unwrap_or_default());
                    }
                }
                _ => {}
            }
        }
    }
    Scan {
        units,
        retrievals: calls
            .into_iter()
            .map(|(id, uses)| (id, uses.len()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn big(tag: &str) -> String {
        (0..200)
            .map(|n| format!("{tag} line {n}\n"))
            .collect::<Vec<_>>()
            .concat()
    }

    #[test]
    fn slots_point_at_the_text() {
        let string = Slot {
            message: 1,
            block: 2,
            item: None,
        };
        assert_eq!(string.pointer(), "/messages/1/content/2/content");
        let item = Slot {
            item: Some(3),
            ..string
        };
        assert_eq!(item.pointer(), "/messages/1/content/2/content/3/text");
    }

    #[test]
    fn only_eligible_tool_results_are_read() {
        let rules = PreviewRules {
            min_tokens: 10,
            ..PreviewRules::default()
        };
        let body = json!({
            "system": [{"type": "text", "text": big("system")}],
            "messages": [
                {"role": "user", "content": big("typed")},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": big("thought")},
                    {"type": "text", "text": big("assistant")},
                    {"type": "tool_use", "id": "t1", "name": "Bash", "input": {}},
                    {"type": "tool_use", "id": "t2", "name": "Read", "input": {}},
                    {"type": "tool_use", "id": "t3", "name": "mcp__tokred__tokred_retrieve",
                     "input": {"id": " abc "}},
                    {"type": "tool_use", "id": "t4", "name": "WebFetch", "input": {}},
                    {"type": "tool_use", "id": "t5", "name": "Grep", "input": {}},
                    {"type": "tool_use", "id": "t6", "name": "Read", "input": {"id": "x"}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": big("old")},
                    {"type": "tool_result", "tool_use_id": "t2", "is_error": true,
                     "content": big("error")},
                    {"type": "tool_result", "tool_use_id": "t3", "content": big("retrieved")},
                    {"type": "tool_result", "tool_use_id": "t4", "content": [
                        {"type": "text", "text": big("mixed")},
                        {"type": "image", "source": {"type": "base64", "data": "AAAA"}}
                    ]},
                    {"type": "tool_result", "tool_use_id": "t5"},
                    {"type": "text", "text": big("reminder")}
                ]},
                {"role": "assistant", "content": [
                    {"type": "tool_use", "id": "t7", "name": "mcp__tokred__tokred_retrieve",
                     "input": {"id": "abc"}},
                    {"type": "tool_use", "id": "t7", "name": "mcp__tokred__tokred_retrieve",
                     "input": {"id": "abc"}},
                    {"type": "tool_use", "id": "t8", "name": "Glob", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t8", "content": [
                        {"type": "text", "text": big("first")},
                        {"type": "text", "text": "small", "cache_control": {"type": "ephemeral"}}
                    ]},
                    {"type": "tool_result", "tool_use_id": "missing", "content": big("orphan")}
                ]}
            ]
        });
        let scan = scan(&body, &rules);
        let summary: Vec<(String, String, bool, bool)> = scan
            .units
            .iter()
            .map(|u| {
                (
                    u.slot.pointer(),
                    u.tool.clone(),
                    u.in_last,
                    u.candidate.is_some(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "/messages/2/content/0/content".to_owned(),
                    "Bash".to_owned(),
                    false,
                    false
                ),
                (
                    "/messages/4/content/0/content/0/text".to_owned(),
                    "Glob".to_owned(),
                    true,
                    true
                ),
                (
                    "/messages/4/content/0/content/1/text".to_owned(),
                    "Glob".to_owned(),
                    true,
                    false
                ),
                (
                    "/messages/4/content/1/content".to_owned(),
                    UNKNOWN_TOOL.to_owned(),
                    true,
                    true
                ),
            ]
        );
        assert_eq!(scan.units[0].hash, content_hash(&big("old")));
        // Repeated tool_use ids count once; ids are trimmed; other tools' `id` inputs are ignored.
        assert_eq!(scan.retrievals, HashMap::from([("abc".to_owned(), 2)]));
    }

    #[test]
    fn bodies_without_messages_or_with_an_assistant_last() {
        let rules = PreviewRules::default();
        assert!(scan(&json!({"messages": "no"}), &rules).units.is_empty());
        assert!(scan(&json!([]), &rules).units.is_empty());
        let body = json!({"messages": [
            {"role": "user", "content": [{"type": "tool_result", "content": big("x")}]},
            {"role": "assistant", "content": "prefill"}
        ]});
        let scan = scan(&body, &rules);
        assert_eq!(scan.units.len(), 1);
        assert!(!scan.units[0].in_last, "only a last *user* message is new");
    }

    #[test]
    fn the_retrieval_tool_is_recognized_by_suffix() {
        assert!(is_retrieve_tool("tokred_retrieve"));
        assert!(is_retrieve_tool("mcp__tokred__tokred_retrieve"));
        assert!(!is_retrieve_tool("Read"));
    }
}
