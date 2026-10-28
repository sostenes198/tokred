//! The line printed by `tokred statusline` (spec 5.6, channel 2).
//!
//! Claude Code runs the `statusLine` command after each assistant message (debounced at
//! 300 ms) with a JSON object on stdin and shows what it prints; ANSI colors are allowed. The
//! fields read here are documented at <https://code.claude.com/docs/en/statusline>:
//! `session_id`, `model.id`, `model.display_name`, `cost.total_cost_usd` (Claude Code's own
//! estimate at list prices; it resets to zero when `/clear` starts a new session) and
//! `prompt_cache.warm` (Claude Code 2.1.251 or later).
//!
//! Rendering never marks a suggestion as shown: the status line is redrawn constantly, and
//! marking would hide the suggestion from the `UserPromptSubmit` message channel.

use serde_json::Value;
use tokred_core::config::SuggestChannel;
use tokred_core::prompts::{PromptSummary, SuggestionRecord};

/// What tokred reads from the status line's stdin. Every field is optional.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatuslineInput {
    /// `session_id`.
    pub session_id: Option<String>,
    /// `model.id`.
    pub model_id: Option<String>,
    /// `model.display_name`.
    pub model_name: Option<String>,
    /// `cost.total_cost_usd`.
    pub total_cost_usd: Option<f64>,
    /// `prompt_cache.warm`.
    pub cache_warm: Option<bool>,
}

impl StatuslineInput {
    /// Reads the stdin payload; anything unreadable gives empty fields (the status line must
    /// never fail).
    #[must_use]
    pub fn parse(payload: &[u8]) -> Self {
        let value: Value = serde_json::from_slice(payload).unwrap_or(Value::Null);
        let text = |pointer: &str| {
            value
                .pointer(pointer)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        Self {
            session_id: text("/session_id"),
            model_id: text("/model/id"),
            model_name: text("/model/display_name"),
            total_cost_usd: value
                .pointer("/cost/total_cost_usd")
                .and_then(Value::as_f64),
            cache_warm: value.pointer("/prompt_cache/warm").and_then(Value::as_bool),
        }
    }
}

/// A couple of figures about the session shown when there is no suggestion.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SessionFigures {
    /// Prompts summarized so far.
    pub prompts: u32,
    /// Of those, the simple ones.
    pub simple_prompts: u32,
    /// The session's cost so far, USD (from `cost.total_cost_usd`).
    pub cost_usd: Option<f64>,
    /// The main loop's prompt cache is warm (`prompt_cache.warm`).
    pub cache_warm: Option<bool>,
}

impl SessionFigures {
    /// Counts the session's prompt summaries and takes the cost and cache state from the
    /// status line input.
    #[must_use]
    pub fn new(prompts: &[PromptSummary], input: &StatuslineInput) -> Self {
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Self {
            prompts: count(prompts.len()),
            simple_prompts: count(prompts.iter().filter(|p| p.simple).count()),
            cost_usd: input.total_cost_usd,
            cache_warm: input.cache_warm,
        }
    }
}

/// Suggestions appear in the status line (`statusline` or `message`).
#[must_use]
pub const fn shows_in_statusline(channel: SuggestChannel) -> bool {
    matches!(
        channel,
        SuggestChannel::Statusline | SuggestChannel::Message
    )
}

/// Suggestions are also sent as a one-line message to the user (`message`).
#[must_use]
pub const fn shows_as_message(channel: SuggestChannel) -> bool {
    matches!(channel, SuggestChannel::Message)
}

const YELLOW: &str = "\x1b[33m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// One short line: the suggestion when there is one (e.g. `tokred: simple over the last 4
/// prompts · Sonnet would save ~$0.42 (/model sonnet) · cache cold`), else the session's
/// figures (`tokred · 5 prompts, 3 simple · $1.23`). ANSI colors only when `color` is set.
#[must_use]
pub fn statusline_text(
    suggestion: Option<&SuggestionRecord>,
    figures: &SessionFigures,
    color: bool,
) -> String {
    let (line, tint) = match suggestion {
        Some(suggestion) => (suggestion_line(suggestion, figures), YELLOW),
        None => (figures_line(figures), DIM),
    };
    if color {
        format!("{tint}{line}{RESET}")
    } else {
        line
    }
}

fn suggestion_line(suggestion: &SuggestionRecord, figures: &SessionFigures) -> String {
    let detail: Value = serde_json::from_str(&suggestion.detail).unwrap_or(Value::Null);
    let prompts = detail.get("prompts").and_then(Value::as_u64);
    let family = detail.get("target_family").and_then(Value::as_str);
    let saving = detail.get("saving_usd").and_then(Value::as_f64);
    let mut line = match (prompts, family, saving) {
        (Some(n), Some(family), Some(saving)) => format!(
            "tokred: simple over the last {n} prompt{} · {} would save ~${saving:.2} (/model {family})",
            if n == 1 { "" } else { "s" },
            capitalized(family),
        ),
        _ => suggestion.message.clone(),
    };
    if figures.cache_warm == Some(false) {
        line.push_str(" · cache cold");
    }
    line
}

fn figures_line(figures: &SessionFigures) -> String {
    let mut parts = vec!["tokred".to_owned()];
    if figures.prompts > 0 {
        parts.push(format!(
            "{} prompt{}, {} simple",
            figures.prompts,
            if figures.prompts == 1 { "" } else { "s" },
            figures.simple_prompts
        ));
    }
    if let Some(cost) = figures.cost_usd {
        parts.push(format!("${cost:.2}"));
    }
    parts.join(" · ")
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn suggestion(detail: &str) -> SuggestionRecord {
        SuggestionRecord {
            id: "p:suggestion".into(),
            session_id: "s".into(),
            kind: "cheaper_model".into(),
            message: "tokred: your last 4 prompts were simple; claude-sonnet-5-5 would have cost \
                      ~$0.42 less (switch with /model sonnet)."
                .into(),
            detail: detail.into(),
            ..SuggestionRecord::default()
        }
    }

    #[test]
    fn the_documented_fields_are_read_and_garbage_is_tolerated() {
        let payload = json!({
            "session_id": "abc",
            "model": {"id": "claude-opus-5-5", "display_name": "Opus"},
            "cost": {"total_cost_usd": 1.5, "total_duration_ms": 45000},
            "prompt_cache": {"warm": false, "ttl": "1h"},
            "workspace": {"current_dir": "/x"}
        });
        let input = StatuslineInput::parse(payload.to_string().as_bytes());
        assert_eq!(input.session_id.as_deref(), Some("abc"));
        assert_eq!(input.model_id.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(input.model_name.as_deref(), Some("Opus"));
        assert_eq!(input.total_cost_usd, Some(1.5));
        assert_eq!(input.cache_warm, Some(false));

        assert_eq!(
            StatuslineInput::parse(b"not json"),
            StatuslineInput::default()
        );
        assert_eq!(StatuslineInput::parse(b""), StatuslineInput::default());
        let odd = StatuslineInput::parse(br#"{"session_id":"","cost":{"total_cost_usd":"x"}}"#);
        assert_eq!(odd, StatuslineInput::default());
    }

    #[test]
    fn channels() {
        assert!(!shows_in_statusline(SuggestChannel::Off));
        assert!(!shows_in_statusline(SuggestChannel::Stats));
        assert!(shows_in_statusline(SuggestChannel::Statusline));
        assert!(shows_in_statusline(SuggestChannel::Message));
        assert!(shows_as_message(SuggestChannel::Message));
        assert!(!shows_as_message(SuggestChannel::Statusline));
    }

    #[test]
    fn a_suggestion_is_one_short_line() {
        let s = suggestion(r#"{"prompts":4,"target_family":"sonnet","saving_usd":0.4213}"#);
        let figures = SessionFigures::default();
        let line = statusline_text(Some(&s), &figures, false);
        assert_eq!(
            line,
            "tokred: simple over the last 4 prompts · Sonnet would save ~$0.42 (/model sonnet)"
        );
        assert!(!line.contains('\n') && !line.contains('\x1b'));

        let cold = SessionFigures {
            cache_warm: Some(false),
            ..figures
        };
        let one = suggestion(r#"{"prompts":1,"target_family":"haiku","saving_usd":0.1}"#);
        assert_eq!(
            statusline_text(Some(&one), &cold, false),
            "tokred: simple over the last 1 prompt · Haiku would save ~$0.10 (/model haiku) · cache cold"
        );
        let warm = SessionFigures {
            cache_warm: Some(true),
            ..figures
        };
        assert!(!statusline_text(Some(&one), &warm, false).contains("cold"));

        // Without readable evidence the stored message is shown as is.
        let bare = suggestion("{}");
        assert_eq!(statusline_text(Some(&bare), &figures, false), bare.message);
        assert_eq!(
            statusline_text(Some(&suggestion("garbage")), &figures, false),
            bare.message
        );
        assert_eq!(capitalized(""), "");
    }

    #[test]
    fn without_a_suggestion_the_session_figures_are_shown() {
        let prompts = [
            PromptSummary {
                simple: true,
                ..PromptSummary::default()
            },
            PromptSummary::default(),
            PromptSummary {
                simple: true,
                ..PromptSummary::default()
            },
        ];
        let input = StatuslineInput {
            total_cost_usd: Some(1.234),
            cache_warm: Some(true),
            ..StatuslineInput::default()
        };
        let figures = SessionFigures::new(&prompts, &input);
        assert_eq!(
            figures,
            SessionFigures {
                prompts: 3,
                simple_prompts: 2,
                cost_usd: Some(1.234),
                cache_warm: Some(true)
            }
        );
        assert_eq!(
            statusline_text(None, &figures, false),
            "tokred · 3 prompts, 2 simple · $1.23"
        );
        let one = SessionFigures {
            prompts: 1,
            simple_prompts: 0,
            cost_usd: None,
            cache_warm: None,
        };
        assert_eq!(
            statusline_text(None, &one, false),
            "tokred · 1 prompt, 0 simple"
        );
        assert_eq!(
            statusline_text(None, &SessionFigures::default(), false),
            "tokred"
        );
    }

    #[test]
    fn colors_only_when_asked() {
        let s = suggestion(r#"{"prompts":2,"target_family":"sonnet","saving_usd":0.2}"#);
        let figures = SessionFigures::default();
        let line = statusline_text(Some(&s), &figures, true);
        assert!(
            line.starts_with("\x1b[33m") && line.ends_with("\x1b[0m"),
            "{line:?}"
        );
        let plain = statusline_text(None, &figures, true);
        assert_eq!(plain, "\x1b[2mtokred\x1b[0m");
    }
}
