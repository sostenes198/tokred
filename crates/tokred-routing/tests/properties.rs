//! Properties: the main loop is never changed, `observe` never changes a byte, and the same
//! input always gives the same decision.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::*;
use proptest::prelude::*;
use serde_json::{Value, json};
use tokred_core::config::TrafficMode;
use tokred_core::proxy::{Origin, RequestMeta, RequestShape, classify_origin};

const MODELS: &[&str] = &[
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "claude-haiku-5-5",
    "claude-opus-4-8",
    "claude-3-haiku",
];
const TOOLS: &[&str] = &["Read", "Grep", "Glob", "Edit", "Bash", "WebFetch"];
const PROMPTS: &[&str] = &[
    "Find every place that calls the old pricing function",
    "Implement pagination on the users list endpoint please",
    "Planeje a migração do banco de dados para o Postgres 16",
    "Resuma os logs de erro da última noite de deploy",
    "ok",
];
const CLASSES: &[Option<&str>] = &[
    None,
    Some("main"),
    Some("subagent"),
    Some("workflow"),
    Some("compaction"),
    Some("auxiliary"),
    Some("garbage"),
];

fn body_strategy() -> impl Strategy<Value = Value> {
    (
        0..MODELS.len(),
        proptest::sample::subsequence(TOOLS, 0..=TOOLS.len()),
        any::<bool>(),
        0usize..4,
        0..PROMPTS.len(),
        0usize..3000,
    )
        .prop_map(|(model, tools, thinking, messages, prompt, pad)| {
            let mut body = json!({
                "model": MODELS[model],
                "max_tokens": 4096,
                "system": "s".repeat(pad),
                "tools": tools.iter().map(|t| json!({"name": t})).collect::<Vec<_>>(),
                "messages": (0..messages).map(|i| {
                    if i % 2 == 0 {
                        json!({"role": "user", "content": PROMPTS[prompt]})
                    } else {
                        json!({"role": "assistant", "content": "x"})
                    }
                }).collect::<Vec<_>>()
            });
            if thinking {
                body["thinking"] = json!({"type": "adaptive"});
            }
            body
        })
}

fn meta_strategy(with_agent: bool) -> impl Strategy<Value = RequestMeta> {
    (
        proptest::option::of(0u8..3),
        proptest::option::of(0u8..3),
        0..CLASSES.len(),
        any::<bool>(),
        proptest::option::of(prop_oneof![Just("Explore"), Just("Plan"), Just("custom")]),
    )
        .prop_map(
            move |(session, agent, class, compaction, agent_type)| RequestMeta {
                session_id: session.map(|s| format!("s{s}")),
                agent_id: if with_agent {
                    agent.map(|a| format!("a{a}"))
                } else {
                    None
                },
                request_class: CLASSES[class].map(str::to_owned),
                compaction: compaction.then(|| "auto".to_owned()),
                agent_type: agent_type.map(str::to_owned),
                ..RequestMeta::default()
            },
        )
}

fn modes() -> [TrafficMode; 3] {
    [TrafficMode::Off, TrafficMode::Observe, TrafficMode::Enforce]
}

proptest! {
    /// A request with no agent id that does not classify as auxiliary is the main loop (or
    /// compaction): its body is never changed, whatever the mode and whatever came before.
    #[test]
    fn the_main_loop_is_never_changed(
        history in proptest::collection::vec((meta_strategy(true), body_strategy()), 0..6),
        meta in meta_strategy(false),
        body in body_strategy(),
    ) {
        let shape = RequestShape::of(&body);
        let origin = classify_origin(&meta, &shape);
        prop_assume!(matches!(origin, Origin::Main | Origin::Compaction));
        for mode in modes() {
            let (hook, _sink) = new_hook(mode);
            for (n, (meta, body)) in history.iter().enumerate() {
                run(&hook, &Req { meta: meta.clone(), body: body.clone(), now: 100 + i64::try_from(n).unwrap() });
            }
            let req = Req { meta: meta.clone(), body: body.clone(), now: 200 };
            let done = run(&hook, &req);
            prop_assert!(!done.changed);
            prop_assert_eq!(&done.body, &body);
            prop_assert!(done.notes.model_rewritten.is_none());
        }
    }

    /// `observe` decides and records but never changes a byte of any request.
    #[test]
    fn observe_never_changes_a_byte(
        requests in proptest::collection::vec((meta_strategy(true), body_strategy()), 1..10),
    ) {
        let (hook, _sink) = new_hook(TrafficMode::Observe);
        for (n, (meta, body)) in requests.iter().enumerate() {
            let before = serde_json::to_vec(body).unwrap();
            let done = run(&hook, &Req { meta: meta.clone(), body: body.clone(), now: i64::try_from(n).unwrap() });
            prop_assert!(!done.changed);
            prop_assert!(done.notes.model_rewritten.is_none());
            prop_assert_eq!(serde_json::to_vec(&done.body).unwrap(), before);
        }
    }

    /// The same sequence of requests gives the same decisions and the same bodies.
    #[test]
    fn the_same_input_gives_the_same_decision(
        requests in proptest::collection::vec((meta_strategy(true), body_strategy()), 1..8),
    ) {
        let play = || {
            let (hook, sink) = new_hook(TrafficMode::Enforce);
            let bodies: Vec<Value> = requests
                .iter()
                .enumerate()
                .map(|(n, (meta, body))| {
                    run(&hook, &Req { meta: meta.clone(), body: body.clone(), now: i64::try_from(n).unwrap() }).body
                })
                .collect();
            let decisions: Vec<_> = sink
                .decisions()
                .into_iter()
                .map(|d| (d.task_class, d.layer, d.model_to, d.outcome, d.reason))
                .collect();
            (bodies, decisions)
        };
        prop_assert_eq!(play(), play());
    }
}
