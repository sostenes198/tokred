//! `model-guard` (`PreModelSwitch`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use serde_json::{Value, json};
use support::{CWD, Env, note};
use tokred_core::HookEvent;
use tokred_core::config::Mode;
use tokred_hooks::HookResponse;

/// A `/model opus` typed in a session on Sonnet.
fn switch(session: &str, context_tokens: u64, warm: bool) -> Value {
    json!({
        "session_id": session,
        "cwd": CWD,
        "hook_event_name": "PreModelSwitch",
        "from_model": "claude-sonnet-5",
        "to_model": "claude-opus-5",
        "requested_model": "opus",
        "source": "command",
        "context_tokens": context_tokens,
        "prompt_cache_warm": warm,
        "cache_ttl": "5m",
        "estimated_cache_write_usd": 1.1396,
        "pricing": "catalog",
    })
}

fn answer(response: &HookResponse) -> Value {
    serde_json::from_str(response.stdout.as_deref().expect("an answer")).unwrap()
}

fn with_mode(mode: Mode) -> Env {
    let mut env = Env::new();
    env.config.modes.model_guard = mode;
    env
}

#[test]
fn warn_tells_the_user_what_the_switch_costs_and_lets_it_go_ahead() {
    let env = with_mode(Mode::Warn);
    let response = env.run(HookEvent::ModelGuard, &switch("s", 182_340, true));
    let value = answer(&response);
    assert_eq!(
        value.as_object().unwrap().len(),
        1,
        "only systemMessage: {value}"
    );
    let text = value["systemMessage"].as_str().unwrap();
    for part in ["claude-sonnet-5", "claude-opus-5", "182k", "$1.14"] {
        assert!(text.contains(part), "{part} in {text}");
    }
    assert_eq!(note(&response), ("warn".into(), "switch_costly".into(), 0));
}

#[test]
fn on_asks_the_user_to_confirm_and_never_allows_on_its_own() {
    let env = with_mode(Mode::Enforce);
    let response = env.run(HookEvent::ModelGuard, &switch("s", 182_340, true));
    let out = &answer(&response)["hookSpecificOutput"];
    assert_eq!(out["hookEventName"], "PreModelSwitch");
    // "allow" would skip the confirmation Claude Code already shows while the cache is warm.
    assert_eq!(out["permissionDecision"], "ask");
    let reason = out["permissionDecisionReason"].as_str().unwrap();
    for part in ["claude-opus-5", "182k", "$1.14"] {
        assert!(reason.contains(part), "{part} in {reason}");
    }
    assert_eq!(note(&response), ("ask".into(), "switch_costly".into(), 0));
}

#[test]
fn a_small_context_is_left_alone_in_every_mode() {
    for mode in [Mode::Warn, Mode::Enforce] {
        let env = with_mode(mode);
        let response = env.run(HookEvent::ModelGuard, &switch("s", 50_000, true));
        assert_eq!(response.stdout, None, "{mode:?}");
        assert_eq!(note(&response), ("allow".into(), "below_limit".into(), 0));
    }
}

#[test]
fn the_limit_comes_from_the_config() {
    let mut env = with_mode(Mode::Warn);
    env.config.model_guard.min_context_tokens = 1_000;
    let response = env.run(HookEvent::ModelGuard, &switch("s", 50_000, true));
    assert_eq!(note(&response).1, "switch_costly");
    env.config.model_guard.min_context_tokens = 50_001;
    let response = env.run(HookEvent::ModelGuard, &switch("s", 50_000, true));
    assert_eq!(note(&response).1, "below_limit");
}

#[test]
fn a_cold_cache_has_nothing_to_lose_so_the_hook_stays_quiet() {
    let env = with_mode(Mode::Enforce);
    let response = env.run(HookEvent::ModelGuard, &switch("s", 500_000, false));
    assert_eq!(response.stdout, None);
    assert_eq!(note(&response), ("allow".into(), "cache_cold".into(), 0));
}

#[test]
fn switching_to_the_model_already_in_use_is_not_a_switch() {
    let env = with_mode(Mode::Enforce);
    let mut payload = switch("s", 500_000, true);
    payload["to_model"] = json!("claude-sonnet-5");
    let response = env.run(HookEvent::ModelGuard, &payload);
    assert_eq!(response.stdout, None);
    assert_eq!(note(&response), ("allow".into(), "same_model".into(), 0));
}

#[test]
fn without_a_price_the_message_gives_the_size_and_no_dollar_figure() {
    let env = with_mode(Mode::Warn);
    let mut payload = switch("s", 182_340, true);
    payload
        .as_object_mut()
        .unwrap()
        .remove("estimated_cache_write_usd");
    let value = answer(&env.run(HookEvent::ModelGuard, &payload));
    let text = value["systemMessage"].as_str().unwrap();
    assert!(text.contains("182k"), "{text}");
    assert!(!text.contains('$'), "{text}");
}

#[test]
fn a_payload_without_the_cost_fields_is_a_cheap_switch() {
    let env = with_mode(Mode::Enforce);
    let payload = json!({
        "session_id": "s",
        "hook_event_name": "PreModelSwitch",
        "from_model": "a",
        "to_model": "b",
    });
    let response = env.run(HookEvent::ModelGuard, &payload);
    assert_eq!(response.stdout, None);
    assert_eq!(note(&response).1, "below_limit");
}

#[test]
fn off_and_a_missing_session_do_nothing_and_record_nothing() {
    let env = with_mode(Mode::Off);
    let response = env.run(HookEvent::ModelGuard, &switch("s", 500_000, true));
    assert_eq!((response.stdout, response.note), (None, None));

    let env = with_mode(Mode::Enforce);
    let response = env.run(HookEvent::ModelGuard, &switch("", 500_000, true));
    assert_eq!((response.stdout, response.note), (None, None));
}
