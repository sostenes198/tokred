//! Main-loop prompts and model suggestions (spec 5.6). The main loop's model is never changed:
//! tokred only measures each prompt and suggests a cheaper model when the work was simple.
//!
//! This module holds the data shared with the store, the hooks and the CLI; the logic lives in
//! the `tokred-suggest` crate.

use serde::Serialize;

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
