//! Main-loop prompts and model suggestions (spec 5.6). The main loop's model is never changed:
//! tokred only measures each prompt and suggests a cheaper model when the work was simple.
//!
//! This module holds the data shared with the store, the hooks and the CLI; the logic lives in
//! the `tokred-suggest` crate.

use serde::Serialize;
use serde_json::Value;

use crate::error::StoreError;
use crate::types::Timestamp;

/// One user prompt of the main loop and what it cost (row of `prompt_summaries`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PromptSummary {
    /// The prompt id (`x-claude-code-prompt-id` when sent, else derived by the proxy).
    pub prompt_id: String,
    /// Session id.
    pub session_id: String,
    /// When the prompt started.
    pub ts: Timestamp,
    /// The model the main loop used.
    pub model: String,
    /// Main-loop requests the prompt took.
    pub turns: u32,
    /// Names of the tools used, with repeats.
    pub tools: Vec<String>,
    /// Output tokens.
    pub output_tokens: u64,
    /// Thinking tokens.
    pub thinking_tokens: u64,
    /// Real cost in USD, including its subagents.
    pub cost_usd: f64,
    /// Files were edited.
    pub edited: bool,
    /// Subagents started.
    pub subagents: u32,
    /// The prompt met the "simple" criteria.
    pub simple: bool,
    /// Estimated saving on the cheaper model, net of rebuilding the prompt cache, USD.
    pub est_saving_usd: f64,
    /// Extra fields as a JSON object.
    pub detail: String,
}

/// A model suggestion for the main loop (row of `suggestions`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SuggestionRecord {
    /// Unique id.
    pub id: String,
    /// When it was made.
    pub ts: Timestamp,
    /// Session id.
    pub session_id: String,
    /// `cheaper_model`, ...
    pub kind: String,
    /// The one-line message shown to the user.
    pub message: String,
    /// It has been shown (status line, message or stats).
    pub shown: bool,
    /// Extra fields as a JSON object.
    pub detail: String,
}

/// [`SuggestionRecord::kind`] of a "a cheaper model would have done" suggestion.
pub const KIND_CHEAPER_MODEL: &str = "cheaper_model";

/// Lifetime of a default prompt-cache write, in seconds.
pub const CACHE_TTL_5M_SECS: i64 = 300;

/// Lifetime of a 1-hour prompt-cache write, in seconds.
pub const CACHE_TTL_1H_SECS: i64 = 3600;

/// What the proxy saw of a session's main loop (read from `proxy_requests`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MainLoopActivity {
    /// When the latest successful main-loop request arrived.
    pub last_request: Timestamp,
    /// The main loop wrote 1-hour cache entries in this session.
    pub one_hour_cache: bool,
}

impl MainLoopActivity {
    /// How long the main loop's prompt cache stays warm after a request.
    #[must_use]
    pub const fn cache_ttl_secs(&self) -> i64 {
        if self.one_hour_cache {
            CACHE_TTL_1H_SECS
        } else {
            CACHE_TTL_5M_SECS
        }
    }

    /// The prompt cache has expired at `now` (idle for longer than its lifetime), so switching
    /// the model costs nothing extra: the next request rewrites the cache on any model.
    #[must_use]
    pub const fn cache_is_cold(&self, now: Timestamp) -> bool {
        now.0.saturating_sub(self.last_request.0) > self.cache_ttl_secs()
    }
}

/// Persistent prompt summaries (`prompt_summaries`).
pub trait PromptRepo {
    /// Inserts or replaces the summary with the same `prompt_id` (a prompt closed by the idle
    /// flush and continued later is written again under the same id).
    fn upsert_prompt(&self, summary: &PromptSummary) -> Result<(), StoreError>;
    /// Summaries of prompts that started in `[from, to)`, oldest first.
    fn prompts_between(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<PromptSummary>, StoreError>;
    /// Summaries of one session's prompts that started in `[from, to)`, oldest first.
    fn session_prompts(
        &self,
        session_id: &str,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<PromptSummary>, StoreError>;
}

/// Persistent suggestions (`suggestions`) and what the hooks need to show them safely.
pub trait SuggestionRepo {
    /// Stores a suggestion; an existing id is left untouched (including its `shown` flag).
    fn insert_suggestion(&self, suggestion: &SuggestionRecord) -> Result<(), StoreError>;
    /// Suggestions made in `[from, to)`, oldest first.
    fn suggestions_between(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<SuggestionRecord>, StoreError>;
    /// The most recent suggestion of the session that was not shown yet.
    fn latest_unshown_suggestion(
        &self,
        session_id: &str,
    ) -> Result<Option<SuggestionRecord>, StoreError>;
    /// Marks a suggestion shown. Returns `true` only when this call changed it, so two
    /// concurrent callers cannot both show it.
    fn mark_suggestion_shown(&self, id: &str) -> Result<bool, StoreError>;
    /// The session's latest successful main-loop request seen by the proxy, if any.
    fn main_loop_activity(&self, session_id: &str) -> Result<Option<MainLoopActivity>, StoreError>;
}

/// The `tools` column: a JSON array of tool names.
#[must_use]
pub fn encode_tools(tools: &[String]) -> String {
    Value::from(tools.to_vec()).to_string()
}

/// Reads the `tools` column. Anything that is not an array of strings gives an empty list (or
/// skips the non-string entries), so a damaged row never breaks a report.
#[must_use]
pub fn decode_tools(text: &str) -> Vec<String> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(items)) => items
            .into_iter()
            .filter_map(|item| match item {
                Value::String(name) => Some(name),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The text stored in a `detail` column: the value itself when it is a JSON object, else `{}`.
#[must_use]
pub fn normalize_detail(detail: &str) -> String {
    match serde_json::from_str::<Value>(detail) {
        Ok(value @ Value::Object(_)) => value.to_string(),
        _ => "{}".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_round_trip_and_damaged_text_is_tolerated() {
        let tools = vec![
            "Read".to_owned(),
            "Edit \"x\"".to_owned(),
            "Read".to_owned(),
        ];
        let text = encode_tools(&tools);
        assert_eq!(text, r#"["Read","Edit \"x\"","Read"]"#);
        assert_eq!(decode_tools(&text), tools);
        assert_eq!(encode_tools(&[]), "[]");
        assert!(decode_tools("not json").is_empty());
        assert!(decode_tools(r#"{"a":1}"#).is_empty());
        assert_eq!(decode_tools(r#"["Read",3,null,"Bash"]"#), ["Read", "Bash"]);
    }

    #[test]
    fn details_are_kept_only_when_they_are_objects() {
        assert_eq!(normalize_detail(r#"{"a": 1}"#), r#"{"a":1}"#);
        assert_eq!(normalize_detail(""), "{}");
        assert_eq!(normalize_detail("[1]"), "{}");
        assert_eq!(normalize_detail("nope"), "{}");
    }

    #[test]
    fn the_cache_goes_cold_after_its_lifetime() {
        let short = MainLoopActivity {
            last_request: Timestamp(1_000),
            one_hour_cache: false,
        };
        assert_eq!(short.cache_ttl_secs(), 300);
        assert!(
            !short.cache_is_cold(Timestamp(1_300)),
            "exactly the TTL is warm"
        );
        assert!(short.cache_is_cold(Timestamp(1_301)));
        let long = MainLoopActivity {
            one_hour_cache: true,
            ..short
        };
        assert_eq!(long.cache_ttl_secs(), 3600);
        assert!(!long.cache_is_cold(Timestamp(4_000)));
        assert!(long.cache_is_cold(Timestamp(4_601)));
        assert!(!long.cache_is_cold(Timestamp(i64::MIN)), "no overflow");
    }

    #[test]
    fn records_serialize_with_their_field_names() {
        let record = SuggestionRecord {
            id: "x".into(),
            kind: KIND_CHEAPER_MODEL.into(),
            ..SuggestionRecord::default()
        };
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(value["kind"], "cheaper_model");
        let summary = serde_json::to_value(PromptSummary::default()).unwrap();
        assert_eq!(summary["turns"], 0);
    }
}
