//! The cheaper-model suggestion shown on `UserPromptSubmit`.
//!
//! The proxy stores suggestions; this hook only shows one. It does so when `[suggest] channel`
//! is `message`, the session has a suggestion that was never shown, and the moment is safe:
//!
//! - the main loop's prompt cache is already cold (no main-loop request for longer than its
//!   lifetime: 5 minutes, or 1 hour when the session writes 1-hour cache), so switching models
//!   costs no extra cache rebuild; or
//! - nothing ran in the main loop since the last `/compact` (recorded by `session-start`),
//!   whose summary starts a new cache anyway; or
//! - the proxy saw no main-loop request of the session at all.
//!
//! The suggestion goes out as `systemMessage`, which Claude Code shows to the user and never
//! adds to the model's context (never as `additionalContext`). It is shown once: the row is
//! claimed (`shown = 0 -> 1`) before it is printed, so two hooks cannot both print it. Any
//! error or panic here is logged and ignored, and never changes the rest of the answer.
//!
//! `/clear` starts a new session id, so a suggestion made in the session before `/clear` is
//! not looked up afterwards.

use std::panic::{AssertUnwindSafe, catch_unwind};

use tokred_core::Timestamp;
use tokred_core::config::SuggestChannel;
use tokred_core::prompts::{MainLoopActivity, SuggestionRepo};

use crate::error::HookError;
use crate::io::HookInput;
use crate::ports::HookDeps;

/// Why the moment is safe (for logs).
pub(crate) const NO_MAIN_ACTIVITY: &str = "no_main_activity";
/// Why the moment is safe (for logs).
pub(crate) const AFTER_RESET: &str = "after_reset";
/// Why the moment is safe (for logs).
pub(crate) const COLD_CACHE: &str = "cold_cache";

/// The one-line message to show now, if any. Never fails: problems are logged and ignored.
pub(crate) fn suggestion_message(
    input: &HookInput,
    deps: &HookDeps<'_>,
    repo: &dyn SuggestionRepo,
) -> Option<String> {
    match catch_unwind(AssertUnwindSafe(|| pending_message(input, deps, repo))) {
        Ok(Ok(message)) => message,
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "model suggestion skipped (fail-open)");
            None
        }
        Err(_) => {
            tracing::warn!("model suggestion panicked (fail-open)");
            None
        }
    }
}

fn pending_message(
    input: &HookInput,
    deps: &HookDeps<'_>,
    repo: &dyn SuggestionRepo,
) -> Result<Option<String>, HookError> {
    let session = input.session_id.as_str();
    if deps.config.suggest.channel != SuggestChannel::Message
        || session.is_empty()
        || input.agent_id.is_some()
    {
        return Ok(None);
    }
    let Some(suggestion) = repo.latest_unshown_suggestion(session)? else {
        return Ok(None);
    };
    let activity = repo.main_loop_activity(session)?;
    let reset = deps
        .sessions
        .get_session(session)?
        .and_then(|s| s.last_reset_at);
    let Some(why) = safe_moment(activity, reset, deps.clock.now()) else {
        return Ok(None);
    };
    if !repo.mark_suggestion_shown(&suggestion.id)? {
        return Ok(None);
    }
    tracing::debug!(id = %suggestion.id, why, "model suggestion shown");
    Ok(Some(one_line(&suggestion.message)))
}

/// Why switching the main model now would not throw away a warm cache, or `None` when it would.
pub(crate) fn safe_moment(
    activity: Option<MainLoopActivity>,
    last_reset: Option<Timestamp>,
    now: Timestamp,
) -> Option<&'static str> {
    let Some(activity) = activity else {
        return Some(NO_MAIN_ACTIVITY);
    };
    if last_reset.is_some_and(|reset| reset >= activity.last_request) {
        Some(AFTER_RESET)
    } else if activity.cache_is_cold(now) {
        Some(COLD_CACHE)
    } else {
        None
    }
}

/// Line breaks become spaces: a `systemMessage` from tokred is one line.
pub(crate) fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::io::Cursor;
    use std::time::Instant;

    use serde_json::{Value, json};
    use tokred_core::config::Mode;
    use tokred_core::prompts::SuggestionRecord;
    use tokred_core::session::SessionRepo;
    use tokred_core::{Clock, Config, HookEvent, StoreError};
    use tokred_store::Store;

    use super::*;
    use crate::fs::StdFileMeta;
    use crate::{HookResponse, HookRun, handle_with, run_hook_with};

    const NOW: i64 = 1_000_000;

    struct FixedClock(Cell<i64>);

    impl Clock for FixedClock {
        fn now(&self) -> Timestamp {
            Timestamp(self.0.get())
        }
    }

    /// An in-memory repository with failure switches.
    #[derive(Default)]
    struct FakeRepo {
        rows: RefCell<Vec<SuggestionRecord>>,
        activity: Cell<Option<MainLoopActivity>>,
        fail: Cell<bool>,
        panic: Cell<bool>,
        stolen: Cell<bool>,
    }

    impl FakeRepo {
        fn check(&self) -> Result<(), StoreError> {
            assert!(!self.panic.get(), "repository panicked");
            if self.fail.get() {
                return Err(StoreError::backend(std::io::Error::other("db is broken")));
            }
            Ok(())
        }
    }

    impl SuggestionRepo for FakeRepo {
        fn insert_suggestion(&self, suggestion: &SuggestionRecord) -> Result<(), StoreError> {
            self.rows.borrow_mut().push(suggestion.clone());
            Ok(())
        }

        fn suggestions_between(
            &self,
            _from: Timestamp,
            _to: Timestamp,
        ) -> Result<Vec<SuggestionRecord>, StoreError> {
            Ok(self.rows.borrow().clone())
        }

        fn latest_unshown_suggestion(
            &self,
            session_id: &str,
        ) -> Result<Option<SuggestionRecord>, StoreError> {
            self.check()?;
            let rows = self.rows.borrow();
            Ok(rows
                .iter()
                .rev()
                .find(|s| s.session_id == session_id && !s.shown)
                .cloned())
        }

        fn mark_suggestion_shown(&self, id: &str) -> Result<bool, StoreError> {
            if self.stolen.get() {
                return Ok(false);
            }
            let mut rows = self.rows.borrow_mut();
            let row = rows.iter_mut().find(|s| s.id == id && !s.shown);
            Ok(row.map(|s| s.shown = true).is_some())
        }

        fn main_loop_activity(
            &self,
            _session_id: &str,
        ) -> Result<Option<MainLoopActivity>, StoreError> {
            self.check()?;
            Ok(self.activity.get())
        }
    }

    const MESSAGE: &str = "tokred: your last 3 prompts were simple; claude-sonnet-5-5 would \
                           have cost ~$0.42 less (switch with /model sonnet).";

    fn record(id: &str, session: &str) -> SuggestionRecord {
        SuggestionRecord {
            id: id.to_owned(),
            ts: Timestamp(NOW - 600),
            session_id: session.to_owned(),
            kind: "cheaper_model".to_owned(),
            message: MESSAGE.to_owned(),
            shown: false,
            detail: "{}".to_owned(),
        }
    }

    struct Env {
        store: Store,
        files: StdFileMeta,
        clock: FixedClock,
        config: Config,
        repo: FakeRepo,
    }

    impl Env {
        fn new() -> Self {
            let mut config = Config::default();
            config.suggest.channel = SuggestChannel::Message;
            let repo = FakeRepo::default();
            repo.insert_suggestion(&record("p3:suggestion", "s1"))
                .unwrap();
            Self {
                store: Store::open_in_memory().unwrap(),
                files: StdFileMeta::default(),
                clock: FixedClock(Cell::new(NOW)),
                config,
                repo,
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

        /// The main loop's last request was `ago` seconds before now.
        fn active(&self, ago: i64, one_hour_cache: bool) {
            self.repo.activity.set(Some(MainLoopActivity {
                last_request: Timestamp(NOW - ago),
                one_hour_cache,
            }));
        }

        fn submit(&self, payload: &Value) -> HookResponse {
            let bytes = payload.to_string();
            handle_with(
                HookEvent::Route,
                bytes.as_bytes(),
                &self.deps(),
                Some(&self.repo),
                None,
            )
            .unwrap()
        }

        fn shown(&self) -> Vec<bool> {
            let rows = self.repo.suggestions_between(Timestamp(0), Timestamp(NOW));
            rows.unwrap().iter().map(|s| s.shown).collect()
        }
    }

    fn prompt(session: &str, text: &str) -> Value {
        json!({
            "session_id": session, "cwd": "/work", "hook_event_name": "UserPromptSubmit",
            "prompt": text,
        })
    }

    fn answer(response: &HookResponse) -> Value {
        serde_json::from_str(response.stdout.as_deref().unwrap()).unwrap()
    }

    #[test]
    fn safe_moments() {
        let now = Timestamp(NOW);
        let at = |ago: i64, one_hour_cache: bool| {
            Some(MainLoopActivity {
                last_request: Timestamp(NOW - ago),
                one_hour_cache,
            })
        };
        assert_eq!(safe_moment(None, None, now), Some(NO_MAIN_ACTIVITY));
        assert_eq!(safe_moment(at(301, false), None, now), Some(COLD_CACHE));
        assert_eq!(safe_moment(at(300, false), None, now), None, "still warm");
        assert_eq!(
            safe_moment(at(1_800, true), None, now),
            None,
            "1-hour cache"
        );
        assert_eq!(safe_moment(at(3_601, true), None, now), Some(COLD_CACHE));
        // A compaction after the last main-loop request (or in the same second).
        let reset = Some(Timestamp(NOW - 10));
        assert_eq!(safe_moment(at(20, false), reset, now), Some(AFTER_RESET));
        assert_eq!(safe_moment(at(10, false), reset, now), Some(AFTER_RESET));
        assert_eq!(safe_moment(at(5, false), reset, now), None, "work since");
    }

    #[test]
    fn the_message_is_shown_once_as_a_system_message_when_the_cache_is_cold() {
        let env = Env::new();
        env.active(301, false);
        let first = env.submit(&prompt("s1", "ok, thanks"));
        let value = answer(&first);
        assert_eq!(value, json!({"systemMessage": MESSAGE}));
        assert!(
            value.get("hookSpecificOutput").is_none(),
            "never additionalContext"
        );
        assert_eq!(env.shown(), [true]);
        // The route note is unchanged by the suggestion.
        let note = first.note.unwrap();
        assert_eq!((note.decision, note.reason), ("other", "unchanged"));

        let second = env.submit(&prompt("s1", "and now?"));
        assert_eq!(second.stdout, None, "shown once");
    }

    #[test]
    fn nothing_is_shown_while_the_cache_is_warm() {
        let env = Env::new();
        env.active(60, false);
        assert_eq!(env.submit(&prompt("s1", "ok")).stdout, None);
        env.active(1_800, true);
        assert_eq!(env.submit(&prompt("s1", "ok")).stdout, None);
        assert_eq!(env.shown(), [false], "still pending");

        // Later the cache went cold: now it is shown.
        env.active(3_601, true);
        assert!(env.submit(&prompt("s1", "ok")).stdout.is_some());
    }

    #[test]
    fn right_after_a_compaction_the_message_is_shown_even_with_a_warm_cache() {
        let env = Env::new();
        env.active(30, false);
        env.store.mark_reset("s1", Timestamp(NOW - 20)).unwrap();
        let value = answer(&env.submit(&prompt("s1", "continue")));
        assert_eq!(value["systemMessage"], MESSAGE);
    }

    #[test]
    fn the_route_line_and_the_message_travel_separately() {
        let env = Env::new();
        let text = "Implement pagination on the users list endpoint in src/users/list.ts";
        let value = answer(&env.submit(&prompt("s1", text)));
        assert_eq!(value["systemMessage"], MESSAGE);
        let context = value["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(context.contains("tokred-dev"));
        assert!(
            !context.contains("/model"),
            "the suggestion never reaches the model"
        );
    }

    #[test]
    fn the_route_mode_does_not_matter_but_the_channel_does() {
        let mut env = Env::new();
        env.config.modes.route = Mode::Off;
        let response = env.submit(&prompt("s1", "hi"));
        assert_eq!(answer(&response), json!({"systemMessage": MESSAGE}));
        assert!(response.note.is_none());

        for channel in [
            SuggestChannel::Off,
            SuggestChannel::Stats,
            SuggestChannel::Statusline,
        ] {
            let mut env = Env::new();
            env.config.suggest.channel = channel;
            assert_eq!(env.submit(&prompt("s1", "hi")).stdout, None, "{channel:?}");
            assert_eq!(env.shown(), [false], "not marked shown");
        }
    }

    #[test]
    fn other_sessions_subagents_and_unknown_sessions_get_nothing() {
        let env = Env::new();
        assert_eq!(env.submit(&prompt("s2", "hi")).stdout, None);
        let mut inside = prompt("s1", "hi");
        inside["agent_id"] = json!("a1");
        assert_eq!(env.submit(&inside).stdout, None);
        assert_eq!(env.submit(&prompt("", "hi")).stdout, None);
        assert_eq!(env.submit(&json!({"session_id": "s1"})).stdout, None);
        assert_eq!(env.shown(), [false]);
    }

    #[test]
    fn a_broken_or_panicking_repository_changes_nothing_else() {
        let env = Env::new();
        env.repo.fail.set(true);
        let text = "Implement pagination on the users list endpoint in src/users/list.ts";
        let response = env.submit(&prompt("s1", text));
        let value = answer(&response);
        assert!(value.get("systemMessage").is_none());
        assert!(value["hookSpecificOutput"]["additionalContext"].is_string());

        env.repo.fail.set(false);
        env.repo.panic.set(true);
        assert_eq!(env.submit(&prompt("s1", "hi")).stdout, None);
        env.repo.panic.set(false);
        assert_eq!(env.shown(), [false]);
    }

    #[test]
    fn a_suggestion_claimed_elsewhere_is_not_shown_twice() {
        let env = Env::new();
        // Another hook claims the row between this one's read and its claim.
        env.repo.stolen.set(true);
        assert_eq!(env.submit(&prompt("s1", "hi")).stdout, None);
    }

    #[test]
    fn the_real_store_shows_a_pending_suggestion_once() {
        let env = Env::new();
        let store = Store::open_in_memory().unwrap();
        store
            .insert_suggestion(&record("p9:suggestion", "s1"))
            .unwrap();
        let input: HookInput = serde_json::from_value(prompt("s1", "hi")).unwrap();
        let shown = suggestion_message(&input, &env.deps(), &store);
        assert_eq!(
            shown.as_deref(),
            Some(MESSAGE),
            "no main-loop activity: safe"
        );
        assert_eq!(suggestion_message(&input, &env.deps(), &store), None);
        assert!(store.latest_unshown_suggestion("s1").unwrap().is_none());
    }

    #[test]
    fn the_runtime_prints_the_message() {
        let env = Env::new();
        let deps = env.deps();
        let run = HookRun {
            deps: Some(&deps),
            events: Some(&env.store),
            clock: &env.clock,
            start: Instant::now(),
        };
        let payload = prompt("s1", "hi").to_string();
        let mut out = Vec::new();
        run_hook_with(
            Some("route"),
            &mut Cursor::new(payload.into_bytes()),
            &mut out,
            &run,
            Some(&env.repo),
            None,
        );
        let value: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value, json!({"systemMessage": MESSAGE}));
    }

    #[test]
    fn messages_are_one_line() {
        assert_eq!(one_line("a\nb\r\nc"), "a b  c");
        assert_eq!(one_line(MESSAGE), MESSAGE);
    }
}
