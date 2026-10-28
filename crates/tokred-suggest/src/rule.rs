//! When a run of simple prompts turns into a suggestion (spec 5.6).
//!
//! A session's prompts are followed in order. A prompt that is simple **and** would have been
//! cheaper on the next family extends the streak; anything else (a prompt that is not simple, a
//! haiku or unpriced main loop, a change of model) ends it. Once the streak holds
//! `min_simple_prompts` prompts, the saving of the streak, net of rebuilding the cache once (a
//! single switch before its first prompt), must exceed `min_saving_usd`, and the session must
//! not have had a suggestion within `cooldown`. A suggestion ends the streak.

use serde_json::json;
use tokred_core::Timestamp;
use tokred_core::cache::parse_duration;
use tokred_core::config::{SuggestChannel, SuggestConfig};
use tokred_core::pricing::Family;
use tokred_core::prompts::{KIND_CHEAPER_MODEL, SuggestionRecord};

use crate::counterfactual::Counterfactual;

/// Default `min_saving_usd`: below this a suggestion is not worth the interruption.
pub const DEFAULT_MIN_SAVING_USD: f64 = 0.05;

/// Default `cooldown` (`6h`), used when the configured value cannot be read.
pub const DEFAULT_COOLDOWN_SECS: i64 = 6 * 3600;

/// The suggestion rule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SuggestRule {
    /// Suggestions are made at all (`[suggest] channel` is not `off`).
    pub enabled: bool,
    /// Simple prompts in a row before a suggestion (at least 1).
    pub min_simple_prompts: u32,
    /// The streak must save more than this, net of one cache rebuild, USD.
    pub min_saving_usd: f64,
    /// Minimum time between two suggestions in a session, seconds.
    pub cooldown_secs: i64,
}

impl Default for SuggestRule {
    fn default() -> Self {
        Self::from_config(&SuggestConfig::default())
    }
}

impl SuggestRule {
    /// The rule for `[suggest]`: `channel = "off"` disables it, `min_simple_prompts` is at least
    /// 1 and an unreadable `cooldown` falls back to 6 hours. The minimum saving has no key yet
    /// and is [`DEFAULT_MIN_SAVING_USD`].
    #[must_use]
    pub fn from_config(config: &SuggestConfig) -> Self {
        Self {
            enabled: config.channel != SuggestChannel::Off,
            min_simple_prompts: config.min_simple_prompts.max(1),
            min_saving_usd: DEFAULT_MIN_SAVING_USD,
            cooldown_secs: parse_duration(&config.cooldown).unwrap_or(DEFAULT_COOLDOWN_SECS),
        }
    }
}

/// A finished prompt, as the rule sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptOutcome {
    /// The prompt id.
    pub prompt_id: String,
    /// The main loop's model.
    pub model: String,
    /// The prompt met the "simple" criteria.
    pub simple: bool,
    /// The real cost of the main loop, USD.
    pub main_cost_usd: f64,
    /// The main loop on the next cheaper family, when there is one.
    pub counterfactual: Option<Counterfactual>,
}

/// A suggestion the rule made.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// The prompts of the streak, oldest first.
    pub prompt_ids: Vec<String>,
    /// The main loop's model.
    pub model: String,
    /// The cheaper model.
    pub target_model: String,
    /// Its family (the `/model` alias).
    pub target_family: Family,
    /// What the streak would have saved, net of one cache rebuild, USD.
    pub saving_usd: f64,
    /// The cache rebuild charged, USD.
    pub rebuild_usd: f64,
}

impl Suggestion {
    /// The one-line message for the user, e.g. `tokred: your last 4 prompts were simple;
    /// claude-sonnet-5-5 would have cost ~$0.42 less (switch with /model sonnet).`
    #[must_use]
    pub fn message(&self) -> String {
        let prompts = match self.prompt_ids.len() {
            1 => "your last prompt was".to_owned(),
            n => format!("your last {n} prompts were"),
        };
        format!(
            "tokred: {prompts} simple; {} would have cost ~${:.2} less (switch with /model {}).",
            self.target_model,
            self.saving_usd,
            self.target_family.name()
        )
    }

    /// The record to store, made at `now` in `session_id`. Its id is derived from the last
    /// prompt of the streak, so storing it twice keeps one row.
    #[must_use]
    pub fn to_record(&self, session_id: &str, now: Timestamp) -> SuggestionRecord {
        let last = self.prompt_ids.last().map_or("", String::as_str);
        SuggestionRecord {
            id: format!("{last}:suggestion"),
            ts: now,
            session_id: session_id.to_owned(),
            kind: KIND_CHEAPER_MODEL.to_owned(),
            message: self.message(),
            shown: false,
            detail: json!({
                "model": self.model,
                "target_model": self.target_model,
                "target_family": self.target_family.name(),
                "prompts": self.prompt_ids.len(),
                "prompt_ids": self.prompt_ids,
                "saving_usd": self.saving_usd,
                "rebuild_usd": self.rebuild_usd,
            })
            .to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    prompt_id: String,
    gross_usd: f64,
    rebuild_usd: f64,
}

/// The run of simple prompts of one session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Streak {
    model: String,
    target_model: String,
    entries: Vec<Entry>,
    last_suggested: Option<Timestamp>,
    last_suggested_prompt: Option<String>,
}

impl Streak {
    /// Prompts in the current streak.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The streak is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Adds a finished prompt (closed at `now`) and returns a suggestion when the rule fires.
    ///
    /// A prompt evaluated again (closed by the idle flush, then continued) replaces its earlier
    /// evaluation; one that already led to a suggestion is not counted twice.
    pub fn record(
        &mut self,
        outcome: &PromptOutcome,
        rule: &SuggestRule,
        now: Timestamp,
    ) -> Option<Suggestion> {
        if self.last_suggested_prompt.as_deref() == Some(outcome.prompt_id.as_str()) {
            return None;
        }
        if self
            .entries
            .last()
            .is_some_and(|entry| entry.prompt_id == outcome.prompt_id)
        {
            self.entries.pop();
        }
        let Some(cf) = outcome
            .counterfactual
            .as_ref()
            .filter(|cf| outcome.simple && cf.gross_saving(outcome.main_cost_usd) > 0.0)
        else {
            self.entries.clear();
            return None;
        };
        if self.model != outcome.model || self.target_model != cf.target_model {
            self.entries.clear();
            self.model.clone_from(&outcome.model);
            self.target_model.clone_from(&cf.target_model);
        }
        self.entries.push(Entry {
            prompt_id: outcome.prompt_id.clone(),
            gross_usd: cf.gross_saving(outcome.main_cost_usd),
            rebuild_usd: cf.rebuild_usd,
        });

        let cooling = self
            .last_suggested
            .is_some_and(|at| now.0.saturating_sub(at.0) < rule.cooldown_secs);
        if !rule.enabled || self.entries.len() < rule.min_simple_prompts as usize || cooling {
            return None;
        }
        let rebuild_usd = self.entries.first().map_or(0.0, |e| e.rebuild_usd);
        let saving_usd = self.entries.iter().map(|e| e.gross_usd).sum::<f64>() - rebuild_usd;
        if saving_usd.partial_cmp(&rule.min_saving_usd) != Some(std::cmp::Ordering::Greater) {
            return None;
        }
        let prompt_ids: Vec<String> = self.entries.drain(..).map(|e| e.prompt_id).collect();
        self.last_suggested = Some(now);
        self.last_suggested_prompt
            .clone_from(&prompt_ids.last().cloned());
        Some(Suggestion {
            prompt_ids,
            model: outcome.model.clone(),
            target_model: cf.target_model.clone(),
            target_family: cf.target_family,
            saving_usd,
            rebuild_usd,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn rule(min: u32) -> SuggestRule {
        SuggestRule {
            enabled: true,
            min_simple_prompts: min,
            min_saving_usd: 0.05,
            cooldown_secs: 3600,
        }
    }

    fn cf(target: &str, target_usd: f64, rebuild_usd: f64) -> Counterfactual {
        Counterfactual {
            target_model: target.to_owned(),
            target_family: Family::from_model(target).unwrap(),
            target_usd,
            rebuild_usd,
        }
    }

    /// A simple opus prompt that cost 0.30 and would have cost 0.10 on sonnet (+0.05 rebuild).
    fn simple(id: &str) -> PromptOutcome {
        PromptOutcome {
            prompt_id: id.to_owned(),
            model: "claude-opus-5-5".to_owned(),
            simple: true,
            main_cost_usd: 0.30,
            counterfactual: Some(cf("claude-sonnet-5-5", 0.10, 0.05)),
        }
    }

    #[test]
    fn the_rule_follows_the_config() {
        let config = SuggestConfig::default();
        let r = SuggestRule::from_config(&config);
        assert!(r.enabled);
        assert_eq!(r.min_simple_prompts, 3);
        assert_eq!(r.cooldown_secs, 6 * 3600);
        assert!((r.min_saving_usd - DEFAULT_MIN_SAVING_USD).abs() < f64::EPSILON);
        assert_eq!(SuggestRule::default(), r);

        let odd = SuggestConfig {
            channel: SuggestChannel::Off,
            min_simple_prompts: 0,
            cooldown: "soon".to_owned(),
        };
        let r = SuggestRule::from_config(&odd);
        assert!(!r.enabled);
        assert_eq!(r.min_simple_prompts, 1, "at least one prompt");
        assert_eq!(r.cooldown_secs, DEFAULT_COOLDOWN_SECS);
        let quick = SuggestConfig {
            cooldown: "30m".to_owned(),
            ..SuggestConfig::default()
        };
        assert_eq!(SuggestRule::from_config(&quick).cooldown_secs, 1800);
    }

    #[test]
    fn a_suggestion_needs_the_threshold_of_simple_prompts() {
        let mut streak = Streak::default();
        let now = Timestamp(10_000);
        assert!(streak.record(&simple("p1"), &rule(3), now).is_none());
        assert!(streak.record(&simple("p2"), &rule(3), now).is_none());
        assert_eq!(streak.len(), 2);
        let s = streak.record(&simple("p3"), &rule(3), now).unwrap();
        assert_eq!(s.prompt_ids, ["p1", "p2", "p3"]);
        // 3 x (0.30 - 0.10) - one rebuild of 0.05.
        assert!((s.saving_usd - 0.55).abs() < 1e-12, "{}", s.saving_usd);
        assert!((s.rebuild_usd - 0.05).abs() < 1e-12);
        assert_eq!(s.target_family, Family::Sonnet);
        assert_eq!(
            s.message(),
            "tokred: your last 3 prompts were simple; claude-sonnet-5-5 would have cost ~$0.55 \
             less (switch with /model sonnet)."
        );
        assert!(streak.is_empty(), "a suggestion ends the streak");
    }

    #[test]
    fn a_prompt_that_is_not_simple_or_not_cheaper_breaks_the_streak() {
        let mut streak = Streak::default();
        let now = Timestamp(1);
        streak.record(&simple("p1"), &rule(2), now);
        let not_simple = PromptOutcome {
            simple: false,
            ..simple("p2")
        };
        assert!(streak.record(&not_simple, &rule(2), now).is_none());
        assert!(streak.is_empty());

        streak.record(&simple("p3"), &rule(2), now);
        let haiku = PromptOutcome {
            counterfactual: None,
            ..simple("p4")
        };
        assert!(
            streak.record(&haiku, &rule(2), now).is_none(),
            "never for haiku"
        );
        assert!(streak.is_empty());

        streak.record(&simple("p5"), &rule(2), now);
        let dearer = PromptOutcome {
            counterfactual: Some(cf("claude-sonnet-5-5", 0.40, 0.0)),
            ..simple("p6")
        };
        assert!(streak.record(&dearer, &rule(2), now).is_none());
        assert!(streak.is_empty());
    }

    #[test]
    fn a_change_of_model_starts_a_new_streak() {
        let mut streak = Streak::default();
        let now = Timestamp(1);
        streak.record(&simple("p1"), &rule(2), now);
        let sonnet = PromptOutcome {
            model: "claude-sonnet-5-5".to_owned(),
            counterfactual: Some(cf("claude-haiku-5-5", 0.05, 0.01)),
            ..simple("p2")
        };
        assert!(streak.record(&sonnet, &rule(2), now).is_none());
        assert_eq!(streak.len(), 1);
        let s = streak
            .record(
                &PromptOutcome {
                    prompt_id: "p3".to_owned(),
                    ..sonnet
                },
                &rule(2),
                now,
            )
            .unwrap();
        assert_eq!(s.prompt_ids, ["p2", "p3"]);
        assert_eq!(s.target_model, "claude-haiku-5-5");
        assert!(s.message().contains("/model haiku"));
    }

    #[test]
    fn the_saving_must_exceed_the_minimum() {
        let mut streak = Streak::default();
        let now = Timestamp(1);
        // 2 x 0.20 gross - 0.40 rebuild = 0: not above the minimum, the streak keeps growing.
        let costly = |id: &str| PromptOutcome {
            counterfactual: Some(cf("claude-sonnet-5-5", 0.10, 0.40)),
            ..simple(id)
        };
        assert!(streak.record(&costly("p1"), &rule(2), now).is_none());
        assert!(streak.record(&costly("p2"), &rule(2), now).is_none());
        assert_eq!(streak.len(), 2);
        // A third prompt adds 0.20: 0.60 - 0.40 = 0.20 > 0.05.
        let s = streak.record(&costly("p3"), &rule(2), now).unwrap();
        assert!((s.saving_usd - 0.20).abs() < 1e-12, "{}", s.saving_usd);
        assert!(s.message().starts_with("tokred: your last 3 prompts"));

        // A NaN saving never passes.
        let mut streak = Streak::default();
        let nan = PromptOutcome {
            counterfactual: Some(cf("claude-sonnet-5-5", 0.10, f64::NAN)),
            ..simple("n1")
        };
        assert!(streak.record(&nan, &rule(1), now).is_none());
    }

    #[test]
    fn the_cooldown_holds_back_the_next_suggestion() {
        let mut streak = Streak::default();
        assert!(
            streak
                .record(&simple("p1"), &rule(1), Timestamp(0))
                .is_some()
        );
        assert!(
            streak
                .record(&simple("p2"), &rule(1), Timestamp(3599))
                .is_none()
        );
        assert_eq!(streak.len(), 1, "the streak keeps counting");
        let s = streak
            .record(&simple("p3"), &rule(1), Timestamp(3600))
            .unwrap();
        assert_eq!(s.prompt_ids, ["p2", "p3"]);
        assert_eq!(
            s.message(),
            "tokred: your last 2 prompts were simple; claude-sonnet-5-5 would have cost ~$0.35 \
             less (switch with /model sonnet)."
        );
    }

    #[test]
    fn a_disabled_rule_never_suggests() {
        let mut streak = Streak::default();
        let off = SuggestRule {
            enabled: false,
            ..rule(1)
        };
        assert!(streak.record(&simple("p1"), &off, Timestamp(0)).is_none());
        assert_eq!(streak.len(), 1);
    }

    #[test]
    fn a_prompt_evaluated_again_replaces_its_entry_and_is_not_counted_twice() {
        let mut streak = Streak::default();
        let now = Timestamp(0);
        streak.record(&simple("p1"), &rule(3), now);
        streak.record(&simple("p1"), &rule(3), now);
        assert_eq!(streak.len(), 1);
        // Continued and no longer simple: the streak ends.
        let grown = PromptOutcome {
            simple: false,
            ..simple("p1")
        };
        streak.record(&grown, &rule(3), now);
        assert!(streak.is_empty());

        let mut streak = Streak::default();
        let one = SuggestRule {
            cooldown_secs: 0,
            ..rule(1)
        };
        let s = streak.record(&simple("p9"), &one, now).unwrap();
        assert_eq!(s.prompt_ids, ["p9"]);
        assert_eq!(
            s.message(),
            "tokred: your last prompt was simple; claude-sonnet-5-5 would have cost ~$0.15 less \
             (switch with /model sonnet)."
        );
        assert!(streak.record(&simple("p9"), &one, now).is_none());
        assert!(
            streak.is_empty(),
            "the suggested prompt is not counted again"
        );
    }

    #[test]
    fn the_record_carries_the_evidence() {
        let mut streak = Streak::default();
        let s = streak
            .record(&simple("s1:7:1"), &rule(1), Timestamp(50))
            .unwrap();
        let record = s.to_record("s1", Timestamp(60));
        assert_eq!(record.id, "s1:7:1:suggestion");
        assert_eq!(
            (record.session_id.as_str(), record.ts),
            ("s1", Timestamp(60))
        );
        assert_eq!(record.kind, "cheaper_model");
        assert!(!record.shown);
        assert_eq!(record.message, s.message());
        let detail: Value = serde_json::from_str(&record.detail).unwrap();
        assert_eq!(detail["model"], "claude-opus-5-5");
        assert_eq!(detail["target_model"], "claude-sonnet-5-5");
        assert_eq!(detail["target_family"], "sonnet");
        assert_eq!(detail["prompts"], 1);
        assert_eq!(detail["prompt_ids"][0], "s1:7:1");
        assert!((detail["saving_usd"].as_f64().unwrap() - 0.15).abs() < 1e-12);

        let empty = Suggestion {
            prompt_ids: Vec::new(),
            ..s
        };
        assert_eq!(empty.to_record("s", Timestamp(0)).id, ":suggestion");
    }
}
