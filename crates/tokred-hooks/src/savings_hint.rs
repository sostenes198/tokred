//! The savings hint shown on `UserPromptSubmit`.
//!
//! Routing and compression start in `observe`: they record what they would do and change
//! nothing. Once a week this hook looks at what the proxy measured and, only when turning one of
//! them on is worth it, tells the user in one line. The week is spent whether or not there is
//! anything to say, so the figures are looked at once a week and a quiet week stays quiet.
//!
//! Like the model suggestion, the line goes out as `systemMessage`: Claude Code shows it to the
//! user and never adds it to the model's context. Any error or panic here is logged and ignored.

use std::panic::{AssertUnwindSafe, catch_unwind};

use tokred_core::Timestamp;
use tokred_core::config::SuggestChannel;
use tokred_core::savings::{HINT_EVERY_SECS, PERIOD_DAYS, SavingsRepo};

use crate::error::HookError;
use crate::io::HookInput;
use crate::ports::HookDeps;
use crate::suggest::one_line;

/// Seconds in a day.
const DAY_SECS: i64 = 24 * 60 * 60;

/// The one-line hint to show now, if any. Never fails: problems are logged and ignored.
pub(crate) fn savings_hint_message(
    input: &HookInput,
    deps: &HookDeps<'_>,
    repo: &dyn SavingsRepo,
) -> Option<String> {
    match catch_unwind(AssertUnwindSafe(|| due_hint(input, deps, repo))) {
        Ok(Ok(message)) => message,
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "savings hint skipped (fail-open)");
            None
        }
        Err(_) => {
            tracing::warn!("savings hint panicked (fail-open)");
            None
        }
    }
}

fn due_hint(
    input: &HookInput,
    deps: &HookDeps<'_>,
    repo: &dyn SavingsRepo,
) -> Result<Option<String>, HookError> {
    let config = deps.config;
    // Without the proxy there are no measurements; a subagent's prompt is not the user's.
    if config.suggest.channel == SuggestChannel::Off
        || !config.suggest.savings_hint
        || !config.proxy.enabled
        || input.session_id.is_empty()
        || input.agent_id.is_some()
    {
        return Ok(None);
    }
    let now = deps.clock.now();
    if !repo.claim_savings_hint(now, HINT_EVERY_SECS)? {
        return Ok(None);
    }
    let figures = repo.savings_figures(Timestamp(now.0 - PERIOD_DAYS * DAY_SECS), now)?;
    let hint = figures.hint(config.routing.mode, config.compression.mode);
    Ok(hint.map(|text| one_line(&text)))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use serde_json::{Value, json};
    use tokred_core::prompts::{SuggestionRecord, SuggestionRepo};
    use tokred_core::savings::SavingsFigures;
    use tokred_core::{Clock, Config, HookEvent, StoreError};
    use tokred_store::Store;

    use super::*;
    use crate::fs::StdFileMeta;
    use crate::{HookRun, handle_with, run_hook_with};

    const NOW: i64 = 10_000_000;

    struct FixedClock(Cell<i64>);

    impl Clock for FixedClock {
        fn now(&self) -> Timestamp {
            Timestamp(self.0.get())
        }
    }

    /// Worth turning routing on: a saving above the threshold.
    fn worth() -> SavingsFigures {
        SavingsFigures {
            days: 7.0,
            routing_decisions: 40,
            routing_changed: 25,
            routing_saving_usd: 3.5,
            ..SavingsFigures::default()
        }
    }

    /// An in-memory savings repository: a weekly slot, the figures and failure switches.
    struct FakeSavings {
        next: Cell<i64>,
        claims: Cell<u32>,
        figures: RefCell<SavingsFigures>,
        fail_claim: Cell<bool>,
        fail_figures: Cell<bool>,
        panic: Cell<bool>,
    }

    impl FakeSavings {
        fn new(figures: SavingsFigures) -> Self {
            Self {
                next: Cell::new(0),
                claims: Cell::new(0),
                figures: RefCell::new(figures),
                fail_claim: Cell::new(false),
                fail_figures: Cell::new(false),
                panic: Cell::new(false),
            }
        }
    }

    fn broken() -> StoreError {
        StoreError::backend(std::io::Error::other("db is broken"))
    }

    impl SavingsRepo for FakeSavings {
        fn savings_figures(
            &self,
            _from: Timestamp,
            _to: Timestamp,
        ) -> Result<SavingsFigures, StoreError> {
            assert!(!self.panic.get(), "repository panicked");
            if self.fail_figures.get() {
                return Err(broken());
            }
            Ok(self.figures.borrow().clone())
        }

        fn claim_savings_hint(&self, now: Timestamp, every_secs: i64) -> Result<bool, StoreError> {
            if self.fail_claim.get() {
                return Err(broken());
            }
            if now.0 < self.next.get() {
                return Ok(false);
            }
            self.claims.set(self.claims.get() + 1);
            self.next.set(now.0 + every_secs);
            Ok(true)
        }
    }

    struct Env {
        store: Store,
        files: StdFileMeta,
        clock: FixedClock,
        config: Config,
        savings: FakeSavings,
    }

    impl Env {
        fn new() -> Self {
            let mut config = Config::default();
            config.proxy.enabled = true;
            Self {
                store: Store::open_in_memory().unwrap(),
                files: StdFileMeta::default(),
                clock: FixedClock(Cell::new(NOW)),
                config,
                savings: FakeSavings::new(worth()),
            }
        }

        fn deps(&self) -> HookDeps<'_> {
            HookDeps {
                clock: &self.clock,
                files: &self.files,
                sessions: &self.store,
                reads: &self.store,
                classes: &self.store,
                cache: &self.store,
                config: &self.config,
            }
        }

        /// What the hint says for this payload.
        fn hint(&self, payload: &Value) -> Option<String> {
            let input = HookInput::parse(payload.to_string().as_bytes()).unwrap();
            savings_hint_message(&input, &self.deps(), &self.savings)
        }

        fn later(&self, days: i64) {
            self.clock.0.set(self.clock.0.get() + days * DAY_SECS);
        }
    }

    fn prompt(session: &str) -> Value {
        json!({
            "session_id": session, "cwd": "/work", "hook_event_name": "UserPromptSubmit",
            "prompt": "ok, thanks",
        })
    }

    #[test]
    fn the_hint_is_shown_once_a_week() {
        let env = Env::new();
        let first = env.hint(&prompt("s1")).unwrap();
        assert!(
            first.contains("routing would have saved about $3.50"),
            "{first}"
        );
        assert!(!first.contains('\n'));
        assert_eq!(env.hint(&prompt("s1")), None, "the same week");
        env.later(6);
        assert_eq!(env.hint(&prompt("s1")), None, "still the same week");
        env.later(2);
        let again = env.hint(&prompt("s2"));
        assert!(again.is_some(), "a new week");
        assert_eq!(env.savings.claims.get(), 2);
    }

    #[test]
    fn nothing_is_said_when_it_is_switched_off_or_not_the_users_prompt() {
        let mut env = Env::new();
        env.config.suggest.channel = SuggestChannel::Off;
        assert_eq!(env.hint(&prompt("s1")), None, "channel off");
        env.config.suggest.channel = SuggestChannel::Stats;
        env.config.suggest.savings_hint = false;
        assert_eq!(env.hint(&prompt("s1")), None, "savings_hint = false");
        env.config.suggest.savings_hint = true;
        env.config.proxy.enabled = false;
        assert_eq!(env.hint(&prompt("s1")), None, "no proxy, no measurements");
        env.config.proxy.enabled = true;
        assert_eq!(env.hint(&prompt("")), None, "no session");
        let mut subagent = prompt("s1");
        subagent["agent_id"] = json!("a1");
        assert_eq!(env.hint(&subagent), None, "a subagent");
        assert_eq!(env.savings.claims.get(), 0, "the week was never spent");
        assert!(env.hint(&prompt("s1")).is_some());
    }

    #[test]
    fn small_figures_say_nothing_but_the_week_is_spent() {
        let env = Env::new();
        *env.savings.figures.borrow_mut() = SavingsFigures::default();
        assert_eq!(env.hint(&prompt("s1")), None);
        assert_eq!(env.savings.claims.get(), 1);
        *env.savings.figures.borrow_mut() = worth();
        assert_eq!(env.hint(&prompt("s1")), None, "evaluated once a week");
        assert_eq!(env.savings.claims.get(), 1);
    }

    #[test]
    fn a_feature_that_is_already_on_is_not_mentioned() {
        let mut env = Env::new();
        env.config.routing.mode = tokred_core::config::TrafficMode::Enforce;
        assert_eq!(env.hint(&prompt("s1")), None);
    }

    #[test]
    fn errors_and_panics_are_swallowed() {
        let env = Env::new();
        env.savings.fail_claim.set(true);
        assert_eq!(env.hint(&prompt("s1")), None, "the claim failed");
        env.savings.fail_claim.set(false);
        env.savings.fail_figures.set(true);
        assert_eq!(env.hint(&prompt("s1")), None, "the figures failed");
        env.later(8);
        env.savings.fail_figures.set(false);
        env.savings.panic.set(true);
        assert_eq!(env.hint(&prompt("s1")), None, "the repository panicked");
    }

    #[test]
    fn a_model_suggestion_goes_first_and_leaves_the_week_free() {
        let mut env = Env::new();
        env.config.suggest.channel = SuggestChannel::Message;
        let message = "tokred: your last 3 prompts were simple; switch with /model sonnet.";
        env.store
            .insert_suggestion(&SuggestionRecord {
                id: "p3:suggestion".to_owned(),
                ts: Timestamp(NOW - 600),
                session_id: "s1".to_owned(),
                kind: "cheaper_model".to_owned(),
                message: message.to_owned(),
                shown: false,
                detail: "{}".to_owned(),
            })
            .unwrap();
        let submit = || {
            let bytes = prompt("s1").to_string();
            let response = handle_with(
                HookEvent::Route,
                bytes.as_bytes(),
                &env.deps(),
                Some(&env.store),
                Some(&env.savings),
            )
            .unwrap();
            serde_json::from_str::<Value>(response.stdout.as_deref().unwrap()).unwrap()
        };
        assert_eq!(submit(), json!({"systemMessage": message}), "one message");
        assert_eq!(env.savings.claims.get(), 0, "the week is still free");
        let second = submit();
        let text = second["systemMessage"].as_str().unwrap();
        assert!(text.contains("routing would have saved"), "{text}");
        assert!(
            second.get("hookSpecificOutput").is_none(),
            "never additionalContext"
        );
        assert_eq!(env.savings.claims.get(), 1);
    }

    #[test]
    fn the_runner_hands_the_hint_to_the_prompt_hook() {
        let env = Env::new();
        let clock = FixedClock(Cell::new(NOW));
        let run = HookRun {
            deps: Some(&env.deps()),
            events: None,
            clock: &clock,
            start: std::time::Instant::now(),
        };
        let mut out = Vec::new();
        let input = prompt("s1").to_string();
        run_hook_with(
            Some("route"),
            &mut input.as_bytes(),
            &mut out,
            &run,
            None,
            Some(&env.savings),
        );
        let value: Value = serde_json::from_slice(&out).unwrap();
        assert!(
            value["systemMessage"]
                .as_str()
                .unwrap()
                .contains("tokred (estimate)")
        );
    }
}
