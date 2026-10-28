//! `session-start` and `session-check`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

mod support;

use serde_json::{Value, json};
use support::{CWD, Env, note};
use tokred_core::cache::CacheRepo;
use tokred_core::config::Mode;
use tokred_core::session::SessionRepo;
use tokred_core::{HookEvent, Timestamp};
use tokred_hooks::{HookError, handle, last_context_tokens};

fn start(session: &str, source: &str) -> Value {
    json!({"session_id": session, "cwd": CWD, "hook_event_name": "SessionStart", "source": source})
}

#[test]
fn session_start_registers_the_session_and_prints_nothing() {
    let env = Env::new();
    let response = env.run(HookEvent::SessionStart, &start("s", "startup"));
    assert_eq!(response.stdout, None);
    assert_eq!(note(&response), ("none".into(), "startup".into(), 0));
    let session = env.store.get_session("s").unwrap().unwrap();
    assert_eq!(session.project.as_deref(), Some(CWD));
    assert_eq!(session.last_reset_at, None);
}

#[test]
fn clear_and_compact_mark_a_cut_but_resume_startup_and_fork_do_not() {
    let env = Env::new();
    for (source, cut) in [
        ("startup", false),
        ("resume", false),
        ("fork", false),
        ("clear", true),
        ("compact", true),
        ("from-the-future", false),
    ] {
        let session = format!("s-{source}");
        let response = env.run(HookEvent::SessionStart, &start(&session, source));
        let expected_reason = if source == "from-the-future" {
            "unknown"
        } else {
            source
        };
        assert_eq!(note(&response).1, expected_reason);
        let stored = env.store.get_session(&session).unwrap().unwrap();
        assert_eq!(stored.last_reset_at.is_some(), cut, "{source}");
    }
}

#[test]
fn a_cut_moves_with_the_clock_and_a_missing_source_is_unknown() {
    let env = Env::new();
    env.run(HookEvent::SessionStart, &start("s", "compact"));
    env.advance(60);
    env.run(HookEvent::SessionStart, &start("s", "clear"));
    let session = env.store.get_session("s").unwrap().unwrap();
    assert_eq!(session.last_reset_at, Some(Timestamp(1_000_060)));
    assert_eq!(session.started_at, Timestamp(1_000_000));

    let no_source = json!({"session_id": "x", "hook_event_name": "SessionStart"});
    assert_eq!(
        note(&env.run(HookEvent::SessionStart, &no_source)).1,
        "unknown"
    );
    let no_session = json!({"hook_event_name": "SessionStart", "source": "clear"});
    assert_eq!(env.run(HookEvent::SessionStart, &no_session).note, None);
}

#[test]
fn session_start_errors_are_reported_to_the_runtime() {
    let env = Env::new();
    let payload = start("s", "clear").to_string();
    assert!(handle(HookEvent::SessionStart, payload.as_bytes(), &env.deps()).is_ok());
    assert!(matches!(
        handle(HookEvent::SessionStart, b"[", &env.deps()),
        Err(HookError::Input(_))
    ));
}

// ---------------------------------------------------------------- session-check

const TRANSCRIPT: &str = "/t/session.jsonl";

fn assistant(input: u64, cache_read: u64, cache_creation: u64) -> String {
    json!({
        "type": "assistant", "isSidechain": false,
        "message": {"model": "claude-sonnet-5-5", "usage": {
            "input_tokens": input, "output_tokens": 10,
            "cache_read_input_tokens": cache_read, "cache_creation_input_tokens": cache_creation,
        }},
    })
    .to_string()
}

fn transcript(lines: &[String]) -> Vec<u8> {
    let mut text = lines.join("\n");
    text.push('\n');
    text.into_bytes()
}

fn stop(session: &str) -> Value {
    json!({
        "session_id": session, "transcript_path": TRANSCRIPT, "cwd": CWD,
        "hook_event_name": "Stop", "stop_hook_active": false,
    })
}

fn system_message(response: &tokred_hooks::HookResponse) -> Option<String> {
    let value: Value = serde_json::from_str(response.stdout.as_deref()?).ok()?;
    assert_eq!(
        value.as_object().unwrap().len(),
        1,
        "only systemMessage: {value}"
    );
    Some(value["systemMessage"].as_str()?.to_owned())
}

#[test]
fn the_context_size_is_input_plus_cache_read_plus_cache_creation_of_the_last_message() {
    let tail = transcript(&[assistant(1, 1, 1), assistant(2, 100, 20)]);
    assert_eq!(last_context_tokens(&tail), Some(122));
}

#[test]
fn lines_without_usable_usage_are_skipped() {
    let user = json!({"type": "user", "message": {"role": "user", "content": "hi"}}).to_string();
    let sidechain = json!({"type": "assistant", "isSidechain": true,
        "message": {"usage": {"input_tokens": 999_999}}})
    .to_string();
    let synthetic = assistant(0, 0, 0);
    let no_flag =
        json!({"type": "assistant", "message": {"usage": {"input_tokens": 7}}}).to_string();
    let tool_result = json!({"type": "user", "usage": {"input_tokens": 5}}).to_string();
    let no_message = json!({"type": "assistant", "usage": 1}).to_string();
    let not_assistant_usage = json!({"type": "assistant", "message": {"usage": "x"}}).to_string();

    // Walking back from the end: garbage, user, sidechain and synthetic lines are all skipped.
    let tail = transcript(&[
        no_flag,
        "{\"truncated\": \"usage\"".to_owned(),
        user,
        sidechain,
        synthetic,
        tool_result,
        no_message,
        not_assistant_usage,
        "plain text".to_owned(),
    ]);
    assert_eq!(last_context_tokens(&tail), Some(7));
    assert_eq!(last_context_tokens(b""), None);
    assert_eq!(last_context_tokens(b"{\"usage\"}\n"), None);
}

#[test]
fn above_the_limit_the_user_gets_a_system_message_and_nothing_else() {
    let env = Env::new();
    env.files
        .file(TRANSCRIPT, &transcript(&[assistant(3, 100_000, 30_000)]));
    let response = env.run(HookEvent::SessionCheck, &stop("s"));
    let text = system_message(&response).unwrap();
    assert!(text.contains("130k") && text.contains("120k"), "{text}");
    assert!(
        text.contains("/compact") && text.contains("/clear"),
        "{text}"
    );
    assert_eq!(note(&response), ("warn".into(), "context_large".into(), 0));
    // Never blocks the stop and never talks to the model.
    let stdout = response.stdout.unwrap();
    for forbidden in ["decision", "additionalContext", "\"continue\"", "block"] {
        assert!(!stdout.contains(forbidden), "{stdout}");
    }
}

#[test]
fn at_or_below_the_limit_it_stays_quiet() {
    let env = Env::new();
    env.files
        .file(TRANSCRIPT, &transcript(&[assistant(0, 120_000, 0)]));
    let response = env.run(HookEvent::SessionCheck, &stop("s"));
    assert_eq!(response.stdout, None);
    assert_eq!(note(&response), ("allow".into(), "below_limit".into(), 0));
}

#[test]
fn it_does_not_warn_again_on_every_stop() {
    let env = Env::new();
    env.files
        .file(TRANSCRIPT, &transcript(&[assistant(0, 130_000, 0)]));
    assert!(
        env.run(HookEvent::SessionCheck, &stop("s"))
            .stdout
            .is_some()
    );
    for _ in 0..3 {
        env.advance(30);
        let again = env.run(HookEvent::SessionCheck, &stop("s"));
        assert_eq!(again.stdout, None);
        assert_eq!(note(&again).1, "already_warned");
    }
    // Another session is warned on its own.
    assert!(
        env.run(HookEvent::SessionCheck, &stop("other"))
            .stdout
            .is_some()
    );
    // Reading the marker does not count as a cache hit.
    let entries = env.store.list_entries().unwrap();
    assert!(
        entries
            .iter()
            .all(|e| e.hits == 0 && e.kind == "session_warn")
    );

    // It warns again after the context grew by half the limit...
    env.files.file(
        TRANSCRIPT,
        &transcript(&[assistant(0, 130_000 + 60_000, 0)]),
    );
    assert!(
        env.run(HookEvent::SessionCheck, &stop("s"))
            .stdout
            .is_some()
    );
    // ...and after a clear or compact, even at the same size.
    env.files
        .file(TRANSCRIPT, &transcript(&[assistant(0, 130_000, 0)]));
    env.advance(10);
    env.run(HookEvent::SessionStart, &start("s", "compact"));
    assert!(
        env.run(HookEvent::SessionCheck, &stop("s"))
            .stdout
            .is_some()
    );
}

#[test]
fn the_warning_expires_with_the_l1_ttl() {
    let env = Env::new();
    env.files
        .file(TRANSCRIPT, &transcript(&[assistant(0, 130_000, 0)]));
    assert!(
        env.run(HookEvent::SessionCheck, &stop("s"))
            .stdout
            .is_some()
    );
    env.advance(5 * 3600);
    assert!(
        env.run(HookEvent::SessionCheck, &stop("s"))
            .stdout
            .is_some()
    );
}

#[test]
fn a_corrupt_marker_is_treated_as_never_warned() {
    let env = Env::new();
    env.files
        .file(TRANSCRIPT, &transcript(&[assistant(0, 130_000, 0)]));
    env.run(HookEvent::SessionCheck, &stop("s"));
    let mut entry = env.store.list_entries().unwrap().remove(0);
    entry.value = vec![0xff, 0xfe];
    env.store.put_entry(&entry).unwrap();
    assert!(
        env.run(HookEvent::SessionCheck, &stop("s"))
            .stdout
            .is_some()
    );
}

#[test]
fn the_limit_comes_from_the_config() {
    let mut env = Env::new();
    env.config.session_check.context_warn_tokens = 1000;
    env.files
        .file(TRANSCRIPT, &transcript(&[assistant(0, 5000, 0)]));
    let text = system_message(&env.run(HookEvent::SessionCheck, &stop("s"))).unwrap();
    assert!(text.contains("5k") && text.contains("1k"), "{text}");
}

#[test]
fn a_big_transcript_is_read_from_the_tail_and_the_window_grows_when_needed() {
    let env = Env::new();
    // The only assistant line is followed by more than 256 KiB of other lines.
    let filler = json!({"type": "user", "message": {"content": "x".repeat(1000)}}).to_string();
    let mut lines = vec![assistant(0, 150_000, 0)];
    lines.extend(std::iter::repeat_n(filler, 300));
    env.files.file(TRANSCRIPT, &transcript(&lines));
    assert!(
        env.run(HookEvent::SessionCheck, &stop("s"))
            .stdout
            .is_some()
    );
    assert_eq!(
        *env.files.tail_requests.borrow(),
        [256 * 1024, 4 * 1024 * 1024]
    );

    // A small transcript without usage does not trigger a second read.
    let env = Env::new();
    env.files.file(TRANSCRIPT, b"{}\n");
    assert_eq!(env.run(HookEvent::SessionCheck, &stop("s")).stdout, None);
    assert_eq!(env.files.tail_requests.borrow().len(), 1);
}

#[test]
fn session_check_is_quiet_without_a_transcript_session_or_with_the_mode_off() {
    let mut env = Env::new();
    let no_path = json!({"session_id": "s", "hook_event_name": "Stop"});
    assert_eq!(env.run(HookEvent::SessionCheck, &no_path).note, None);
    // The transcript does not exist (yet).
    assert_eq!(env.run(HookEvent::SessionCheck, &stop("s")).note, None);
    env.files
        .file(TRANSCRIPT, &transcript(&[assistant(0, 130_000, 0)]));
    assert_eq!(env.run(HookEvent::SessionCheck, &stop("")).note, None);
    env.config.modes.session_check = Mode::Off;
    assert_eq!(env.run(HookEvent::SessionCheck, &stop("s")).note, None);
    // `enforce` also only warns.
    env.config.modes.session_check = Mode::Enforce;
    let response = env.run(HookEvent::SessionCheck, &stop("s"));
    assert!(system_message(&response).is_some());
}

#[test]
fn session_check_errors_are_reported_to_the_runtime() {
    let env = Env::new();
    env.files
        .file(TRANSCRIPT, &transcript(&[assistant(0, 130_000, 0)]));
    env.files.fail_tail.set(true);
    let payload = stop("s").to_string();
    assert!(matches!(
        handle(HookEvent::SessionCheck, payload.as_bytes(), &env.deps()),
        Err(HookError::Io(_))
    ));
}
