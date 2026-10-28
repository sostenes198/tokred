//! Summaries of the main loop's prompts and suggestions of a cheaper model (spec 5.6). The main
//! loop's model is never changed: tokred only measures and advises, and the user decides.
//!
//! - [`PromptTracker`] is a [`tokred_core::proxy::ProxyHook`]: it follows each session's user
//!   prompts through the proxy and emits a `PromptSummary` per prompt and, after a run of simple
//!   prompts that a cheaper family would have handled for less, a `SuggestionRecord`.
//! - [`SimpleCriteria`] holds the "simple prompt" thresholds and [`counterfactual()`] prices a
//!   prompt on the next cheaper family, including the cost of rebuilding the prompt cache.
//! - [`statusline_text`] renders the line for `tokred statusline`.

mod counterfactual;
mod criteria;
mod rule;
mod statusline;
mod tracker;

pub use counterfactual::{Counterfactual, cheaper_family, counterfactual};
pub use criteria::{PromptFacts, SimpleCriteria, is_edit_tool, is_subagent_tool};
pub use rule::{
    DEFAULT_COOLDOWN_SECS, DEFAULT_MIN_SAVING_USD, PromptOutcome, Streak, SuggestRule, Suggestion,
};
pub use statusline::{
    SessionFigures, StatuslineInput, shows_as_message, shows_in_statusline, statusline_text,
};
pub use tracker::{
    DEFAULT_IDLE_SECS, DEFAULT_MAX_SESSIONS, DEFAULT_MAX_TOOLS, PromptTracker, TrackerSettings,
};
