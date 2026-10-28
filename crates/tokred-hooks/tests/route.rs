//! `route` and `enforce`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

mod support;

use serde_json::{Value, json};
use support::{CWD, Env, context_of, denial, note, tool};
use tokred_core::HookEvent;
use tokred_core::config::Mode;
use tokred_core::session::{ClassificationRepo, TaskClass};

fn prompt(session: &str, text: &str) -> Value {
    json!({
        "session_id": session, "cwd": CWD, "hook_event_name": "UserPromptSubmit",
        "prompt": text,
    })
}

const IMPL: &str = "Implement pagination on the users list endpoint in src/users/list.ts";
const ARCH: &str = "What is the best architecture for the payments module of this system?";

#[test]
fn a_suggestion_is_one_line_and_only_when_the_class_changed() {
    let env = Env::new();
    let first = env.run(HookEvent::Route, &prompt("s", IMPL));
    let line = context_of(&first).unwrap();
    assert_eq!(
        line,
        "tokred: implementation task -> delegate to the tokred-dev subagent if it involves more than one edit."
    );
    assert!(!line.contains('\n'));
    let value: Value = serde_json::from_str(first.stdout.as_deref().unwrap()).unwrap();
    assert_eq!(
        value["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    assert_eq!(
        note(&first),
        ("implementation".into(), "suggested".into(), 0)
    );

    // Same class again: nothing to say.
    let again = env.run(
        HookEvent::Route,
        &prompt("s", "Now also fix the bug in src/users/list.ts"),
    );
    assert_eq!(again.stdout, None);
    assert_eq!(
        note(&again),
        ("implementation".into(), "unchanged".into(), 0)
    );

    // A different class speaks up again.
    let changed = env.run(HookEvent::Route, &prompt("s", ARCH));
    assert!(context_of(&changed).unwrap().contains("tokred-architect"));
    assert_eq!(note(&changed).0, "architecture");

    // Another session starts from scratch.
    let other = env.run(HookEvent::Route, &prompt("t", ARCH));
    assert!(context_of(&other).is_some());
}

#[test]
fn short_prompts_and_follow_ups_do_not_suggest_or_reset_the_class() {
    let env = Env::new();
    env.run(HookEvent::Route, &prompt("s", IMPL));
    for follow_up in ["ok", "yes, go ahead", "fix the login bug"] {
        let response = env.run(HookEvent::Route, &prompt("s", follow_up));
        assert_eq!(response.stdout, None, "{follow_up}");
        assert_eq!(note(&response), ("other".into(), "unchanged".into(), 0));
    }
    // The last actionable class is still implementation, so no repeated suggestion.
    let last = env.store.last_classification("s").unwrap().unwrap();
    assert_eq!(last.class, TaskClass::Implementation);
    let again = env.run(
        HookEvent::Route,
        &prompt("s", "Implement the same for the orders endpoint now"),
    );
    assert_eq!(again.stdout, None);
}

#[test]
fn the_classification_is_stored_with_the_prompt_length() {
    let env = Env::new();
    env.run(HookEvent::Route, &prompt("s", IMPL));
    let stored = env.store.last_classification("s").unwrap().unwrap();
    assert_eq!(stored.prompt_len, IMPL.chars().count() as u64);
    assert_eq!(stored.ts.0, 1_000_000);
}

#[test]
fn route_is_quiet_when_off_or_when_it_lacks_a_session_or_prompt() {
    let mut env = Env::new();
    assert_eq!(env.run(HookEvent::Route, &prompt("", IMPL)).note, None);
    let no_prompt = json!({"session_id": "s", "hook_event_name": "UserPromptSubmit"});
    assert_eq!(env.run(HookEvent::Route, &no_prompt).note, None);
    env.config.modes.route = Mode::Off;
    let off = env.run(HookEvent::Route, &prompt("s", IMPL));
    assert_eq!((off.stdout, off.note), (None, None));
    assert_eq!(env.store.last_classification("s").unwrap(), None);
}

#[test]
fn enforce_mode_of_route_behaves_like_warn() {
    let mut env = Env::new();
    env.config.modes.route = Mode::Enforce;
    let response = env.run(HookEvent::Route, &prompt("s", IMPL));
    assert!(context_of(&response).is_some());
    assert_eq!(denial(&response), None);
}

fn agent(session: &str, subagent: &str) -> Value {
    tool(
        session,
        "Agent",
        json!({"subagent_type": subagent, "description": "d", "prompt": "p"}),
    )
}

#[test]
fn enforce_allows_a_matching_agent_silently() {
    let env = Env::new();
    env.run(HookEvent::Route, &prompt("s", IMPL));
    let response = env.run(HookEvent::Enforce, &agent("s", "tokred-dev"));
    assert_eq!(response.stdout, None);
    assert_eq!(note(&response), ("allow".into(), "agent_matches".into(), 0));

    // Architecture and planning are both opus agents: either fits either class.
    env.run(HookEvent::Route, &prompt("s", ARCH));
    let planner = env.run(HookEvent::Enforce, &agent("s", "tokred-planner"));
    assert_eq!(note(&planner).1, "agent_matches");
}

#[test]
fn a_mismatch_warns_by_default_and_names_the_right_agent() {
    let env = Env::new();
    env.run(HookEvent::Route, &prompt("s", IMPL));
    let response = env.run(HookEvent::Enforce, &agent("s", "tokred-explorer"));
    assert_eq!(denial(&response), None);
    let text = context_of(&response).unwrap();
    assert!(
        text.contains("tokred-dev") && text.contains("tokred-explorer"),
        "{text}"
    );
    assert_eq!(note(&response), ("warn".into(), "agent_mismatch".into(), 0));
    assert!(!response.stdout.unwrap().contains("permissionDecision"));
}

#[test]
fn a_mismatch_is_denied_in_enforce_mode() {
    let mut env = Env::new();
    env.config.modes.enforce = Mode::Enforce;
    env.run(HookEvent::Route, &prompt("s", IMPL));
    for tool_name in ["Agent", "Task"] {
        let payload = tool(
            "s",
            tool_name,
            json!({"subagent_type": "tokred-summarizer", "prompt": "p"}),
        );
        let response = env.run(HookEvent::Enforce, &payload);
        let reason = denial(&response).unwrap();
        assert!(reason.contains("tokred-dev"), "{reason}");
        assert_eq!(note(&response), ("deny".into(), "agent_mismatch".into(), 0));
    }
}

#[test]
fn foreign_agents_and_unclassified_sessions_are_always_allowed() {
    let mut env = Env::new();
    env.config.modes.enforce = Mode::Enforce;
    env.run(HookEvent::Route, &prompt("s", IMPL));
    for subagent in ["general-purpose", "Explore", ""] {
        let response = env.run(HookEvent::Enforce, &agent("s", subagent));
        assert_eq!(
            (response.stdout.clone(), note(&response).0),
            (None, "allow".into())
        );
    }
    let no_type = tool("s", "Agent", json!({"prompt": "p"}));
    assert_eq!(note(&env.run(HookEvent::Enforce, &no_type)).0, "allow");
    // A tokred agent with nothing classified yet cannot be compared.
    let fresh = env.run(HookEvent::Enforce, &agent("fresh", "tokred-explorer"));
    assert_eq!(
        (fresh.stdout.clone(), note(&fresh).0),
        (None, "allow".into())
    );
}

#[test]
fn enforce_ignores_other_tools_subagents_off_mode_and_missing_sessions() {
    let mut env = Env::new();
    env.config.modes.enforce = Mode::Enforce;
    env.run(HookEvent::Route, &prompt("s", IMPL));
    assert_eq!(
        env.run(HookEvent::Enforce, &tool("s", "Read", json!({})))
            .note,
        None
    );
    assert_eq!(
        env.run(HookEvent::Enforce, &agent("", "tokred-explorer"))
            .note,
        None
    );
    let mut nested = agent("s", "tokred-explorer");
    nested["agent_id"] = json!("a1");
    assert_eq!(env.run(HookEvent::Enforce, &nested).note, None);
    env.config.modes.enforce = Mode::Off;
    assert_eq!(
        env.run(HookEvent::Enforce, &agent("s", "tokred-explorer"))
            .note,
        None
    );
}

#[test]
fn an_unclassifiable_stored_class_allows_anything() {
    let env = Env::new();
    env.store
        .insert_classification(&tokred_core::session::Classification {
            session_id: "s".into(),
            ts: tokred_core::Timestamp(1),
            class: TaskClass::Other,
            prompt_len: 1,
        })
        .unwrap();
    let response = env.run(HookEvent::Enforce, &agent("s", "tokred-explorer"));
    assert_eq!(note(&response).0, "allow");
}
