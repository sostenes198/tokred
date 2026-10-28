//! Hook input (stdin JSON) and output (stdout JSON), as documented at
//! <https://code.claude.com/docs/en/hooks>.
//!
//! Input fields used (all optional except `session_id` for the stateful hooks):
//! `session_id`, `cwd`, `transcript_path`, `agent_id` (present only inside a subagent),
//! `source` (`SessionStart`), `prompt` (`UserPromptSubmit`), `tool_name` and `tool_input`
//! (`PreToolUse`). Unknown fields are ignored.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::HookError;

/// What Claude Code sends on stdin.
#[derive(Debug, Default, Deserialize)]
pub struct HookInput {
    /// Session id (empty when missing).
    #[serde(default)]
    pub session_id: String,
    /// Working directory of the session.
    pub cwd: Option<String>,
    /// Path of the session transcript (JSONL).
    pub transcript_path: Option<String>,
    /// Set only for hooks fired inside a subagent.
    pub agent_id: Option<String>,
    /// `SessionStart`: `startup`, `resume`, `clear`, `compact` or `fork`.
    pub source: Option<String>,
    /// `UserPromptSubmit`: the prompt.
    pub prompt: Option<String>,
    /// `PreToolUse`: the tool name.
    pub tool_name: Option<String>,
    /// `PreToolUse`: the tool arguments.
    pub tool_input: Option<Value>,
}

impl HookInput {
    /// Parses the stdin payload.
    pub fn parse(payload: &[u8]) -> Result<Self, HookError> {
        Ok(serde_json::from_slice(payload)?)
    }

    /// A string argument of the tool.
    #[must_use]
    pub fn tool_str(&self, key: &str) -> Option<&str> {
        self.tool_input.as_ref()?.get(key)?.as_str()
    }

    /// A non-negative integer argument of the tool.
    #[must_use]
    pub fn tool_u64(&self, key: &str) -> Option<u64> {
        self.tool_input.as_ref()?.get(key)?.as_u64()
    }

    /// Key under which per-context state is kept: the session id, plus the subagent id inside
    /// a subagent (its context is separate from the parent's).
    #[must_use]
    pub fn scope(&self) -> String {
        match self.agent_id.as_deref().filter(|id| !id.is_empty()) {
            Some(agent) => format!("{}/{agent}", self.session_id),
            None => self.session_id.clone(),
        }
    }
}

/// `PreToolUse` answer that stops the tool call; `reason` is shown to the model.
#[must_use]
pub fn deny(reason: &str) -> String {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    })
    .to_string()
}

/// An answer that only adds a line of context for the model (the tool call is not decided).
///
/// `permissionDecision: "allow"` is never used: it would skip the user's permission prompt.
#[must_use]
pub fn context(event_name: &str, text: &str) -> String {
    json!({
        "hookSpecificOutput": {
            "hookEventName": event_name,
            "additionalContext": text,
        }
    })
    .to_string()
}

/// An answer that only shows a message to the user (never reaches the model).
#[must_use]
pub fn system_message(text: &str) -> String {
    json!({ "systemMessage": text }).to_string()
}

/// The `UserPromptSubmit` answer: an optional line of context for the model and an optional
/// message for the user (`systemMessage`, which never reaches the model), in one JSON object.
#[must_use]
pub fn prompt_answer(context_line: Option<&str>, message: Option<&str>) -> Option<String> {
    match (context_line, message) {
        (None, None) => None,
        (Some(line), None) => Some(context("UserPromptSubmit", line)),
        (None, Some(text)) => Some(system_message(text)),
        (Some(line), Some(text)) => Some(
            json!({
                "systemMessage": text,
                "hookSpecificOutput": {
                    "hookEventName": "UserPromptSubmit",
                    "additionalContext": line,
                }
            })
            .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> HookInput {
        HookInput::parse(json.as_bytes()).unwrap()
    }

    #[test]
    fn tool_arguments_are_read_by_name_and_type() {
        let input = parse(
            r#"{"session_id":"s","tool_input":{"file_path":"/a","offset":3,"limit":"x","neg":-1}}"#,
        );
        assert_eq!(input.tool_str("file_path"), Some("/a"));
        assert_eq!(input.tool_str("offset"), None);
        assert_eq!(input.tool_u64("offset"), Some(3));
        assert_eq!(input.tool_u64("limit"), None);
        assert_eq!(input.tool_u64("neg"), None);
        assert_eq!(input.tool_u64("missing"), None);
        assert_eq!(parse("{}").tool_str("file_path"), None);
    }

    #[test]
    fn the_scope_separates_subagents_from_their_parent() {
        assert_eq!(parse(r#"{"session_id":"s"}"#).scope(), "s");
        assert_eq!(parse(r#"{"session_id":"s","agent_id":""}"#).scope(), "s");
        assert_eq!(
            parse(r#"{"session_id":"s","agent_id":"a1"}"#).scope(),
            "s/a1"
        );
    }

    #[test]
    fn unknown_fields_are_ignored_and_garbage_is_an_error() {
        assert!(HookInput::parse(br#"{"hook_event_name":"Stop","new_field":[1]}"#).is_ok());
        assert!(HookInput::parse(b"not json").is_err());
        assert!(HookInput::parse(b"").is_err());
        assert!(HookInput::parse(b"[1]").is_err());
    }

    #[test]
    fn answers_follow_the_documented_shapes() {
        let denied: Value = serde_json::from_str(&deny("no")).unwrap();
        assert_eq!(denied["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(denied["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(
            denied["hookSpecificOutput"]["permissionDecisionReason"],
            "no"
        );

        let ctx: Value = serde_json::from_str(&context("UserPromptSubmit", "hi")).unwrap();
        assert_eq!(ctx["hookSpecificOutput"]["additionalContext"], "hi");
        assert!(
            ctx["hookSpecificOutput"]
                .get("permissionDecision")
                .is_none()
        );

        let msg: Value = serde_json::from_str(&system_message("hey")).unwrap();
        assert_eq!(msg, json!({"systemMessage": "hey"}));
    }

    #[test]
    fn a_prompt_answer_carries_context_and_message_separately() {
        assert_eq!(prompt_answer(None, None), None);
        assert_eq!(
            prompt_answer(Some("ctx"), None).unwrap(),
            context("UserPromptSubmit", "ctx")
        );
        assert_eq!(
            prompt_answer(None, Some("note")).unwrap(),
            system_message("note")
        );
        let both: Value =
            serde_json::from_str(&prompt_answer(Some("ctx"), Some("note")).unwrap()).unwrap();
        assert_eq!(
            both,
            json!({
                "systemMessage": "note",
                "hookSpecificOutput": {
                    "hookEventName": "UserPromptSubmit",
                    "additionalContext": "ctx"
                }
            })
        );
    }
}
