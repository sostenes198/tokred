//! The routing hook end to end: sticky subagent decisions, auxiliary requests, guards, modes
//! and the circuit breaker.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::*;
use serde_json::{Value, json};
use tokred_core::Timestamp;
use tokred_core::config::{RoutingConfig, TrafficMode};
use tokred_core::proxy::ProxyHook;
use tokred_routing::{Catalog, RoutingHook, explain};

fn detail(decision: &tokred_core::routing::RoutingDecision) -> Value {
    serde_json::from_str(&decision.detail).unwrap()
}

#[test]
fn a_subagent_is_decided_once_and_the_decision_sticks() {
    let (hook, sink) = new_hook(TrafficMode::Enforce);
    assert_eq!(hook.name(), "routing");
    let main = run(&hook, &main_turn("s1", SONNET, 100));
    assert!(!main.changed && main.notes.decision_id.is_none());

    let first = first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101);
    let done = run(&hook, &first);
    assert!(done.changed);
    assert_eq!(done.body["model"], HAIKU);
    assert_eq!(done.notes.model_rewritten.as_deref(), Some(HAIKU));
    let decisions = sink.decisions();
    assert_eq!(decisions.len(), 1);
    let decision = &decisions[0];
    assert_eq!(
        done.notes.decision_id.as_deref(),
        Some(decision.id.as_str())
    );
    assert_eq!(
        (decision.outcome.as_str(), decision.scope.as_str()),
        ("applied", "agent")
    );
    assert_eq!(
        (decision.layer.as_str(), decision.task_class.as_str()),
        ("tools", "search")
    );
    assert_eq!(decision.agent_id.as_deref(), Some("a1"));
    assert_eq!(
        (decision.model_from.as_str(), decision.model_to.as_str()),
        (SONNET, HAIKU)
    );
    assert!(decision.est_cost_to < decision.est_cost_from);
    assert!(decision.rules_version.starts_with("routing-1+catalog-"));
    assert_eq!(detail(decision)["tier"], "cheap");
    // Everything else in the body is untouched.
    let mut expected = first.body.clone();
    expected["model"] = json!(HAIKU);
    assert_eq!(done.body, expected);

    // Every later request of the agent goes to the same model, with no new decision.
    for turn in 0..3 {
        let later = run(&hook, &later_turn(&first, 102 + turn));
        assert_eq!(later.body["model"], HAIKU);
        assert_eq!(later.notes.decision_id, done.notes.decision_id);
    }
    // A retry of the first request reuses the decision too.
    assert_eq!(run(&hook, &first).body["model"], HAIKU);
    assert_eq!(sink.decisions().len(), 1);
    assert!(explain(decision).contains("applied (mode enforce)"));

    // The same agent id in another session is another agent.
    run(&hook, &main_turn("s2", SONNET, 110));
    run(
        &hook,
        &first_turn("s2", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 111),
    );
    assert_eq!(sink.decisions().len(), 2);
    let status = hook.status(Timestamp(111));
    assert_eq!((status.tracked_agents, status.tracked_sessions), (2, 2));
    assert_eq!(status.mode, "enforce");
    assert_eq!(status.rules_version, hook.rules_version());
    assert_eq!(status.breaker.rewrites, 6);
}

#[test]
fn observe_mode_decides_and_records_but_never_changes_the_body() {
    let (hook, sink) = new_hook(TrafficMode::Observe);
    run(&hook, &main_turn("s1", SONNET, 100));
    let first = first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101);
    let done = run(&hook, &first);
    assert!(!done.changed);
    assert_eq!(
        serde_json::to_vec(&done.body).unwrap(),
        serde_json::to_vec(&first.body).unwrap()
    );
    assert!(done.notes.model_rewritten.is_none());
    let decision = &sink.decisions()[0];
    assert_eq!(
        done.notes.decision_id.as_deref(),
        Some(decision.id.as_str())
    );
    assert_eq!(
        (decision.outcome.as_str(), decision.mode.as_str()),
        ("observed", "observe")
    );
    assert_eq!(decision.model_to, HAIKU, "what it would have done");
    let later_req = later_turn(&first, 102);
    let later = run(&hook, &later_req);
    assert_eq!(later.body, later_req.body);
    assert_eq!(later.notes.decision_id, done.notes.decision_id);
    assert_eq!(sink.decisions().len(), 1);
}

#[test]
fn off_mode_does_nothing() {
    let (hook, sink) = new_hook(TrafficMode::Off);
    run(&hook, &main_turn("s1", SONNET, 100));
    let first = first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101);
    let done = run(&hook, &first);
    assert_eq!(done.body, first.body);
    assert!(done.notes.decision_id.is_none());
    run(&hook, &title("s1", OPUS, 8000, 102));
    assert!(sink.decisions().is_empty());
    assert_eq!(hook.status(Timestamp(0)).mode, "off");
}

#[test]
fn an_agent_first_seen_mid_run_keeps_its_model() {
    let (hook, sink) = new_hook(TrafficMode::Enforce);
    run(&hook, &main_turn("s1", SONNET, 100));
    let first = first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101);
    let later_req = later_turn(&first, 150);
    let later = run(&hook, &later_req);
    assert_eq!(later.body, later_req.body);
    let decision = &sink.decisions()[0];
    assert_eq!(
        (decision.outcome.as_str(), decision.layer.as_str()),
        ("kept", "mid_run")
    );
    assert_eq!(decision.model_to, SONNET);
    assert_eq!(detail(decision)["guard"], "mid_run");
    // It stays kept, with no new decision.
    assert_eq!(run(&hook, &later_turn(&first, 151)).body["model"], SONNET);
    assert_eq!(sink.decisions().len(), 1);
}

#[test]
fn a_first_request_with_a_system_message_is_still_the_first_turn() {
    let (hook, sink) = new_hook(TrafficMode::Enforce);
    run(&hook, &main_turn("s1", SONNET, 100));
    let mut first = first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101);
    first.body["messages"]
        .as_array_mut()
        .unwrap()
        .push(json!({"role": "system", "content": "be brief"}));
    assert_eq!(run(&hook, &first).body["model"], HAIKU);
    assert_eq!(sink.decisions()[0].outcome, "applied");
}

#[test]
fn an_explicitly_chosen_model_is_respected() {
    let (hook, sink) = new_hook(TrafficMode::Enforce);
    // Main loop on Opus, subagent pinned to Sonnet (frontmatter, env var...).
    run(&hook, &main_turn("s1", OPUS, 100));
    let pinned = first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101);
    assert_eq!(run(&hook, &pinned).body, pinned.body);
    // No main-loop request seen yet in this session: cannot tell it was inherited.
    let unknown = first_turn("s2", "a2", SONNET, READ_ONLY, SEARCH_PROMPT, 102);
    assert_eq!(run(&hook, &unknown).body, unknown.body);
    // No session id at all.
    let mut anonymous = first_turn("s3", "a3", SONNET, READ_ONLY, SEARCH_PROMPT, 103);
    anonymous.meta.session_id = None;
    assert_eq!(run(&hook, &anonymous).body, anonymous.body);
    let decisions = sink.decisions();
    assert_eq!(decisions.len(), 3);
    assert!(
        decisions
            .iter()
            .all(|d| d.layer == "explicit" && d.outcome == "kept")
    );
    assert!(decisions[0].reason.contains("chosen on purpose"));
    assert!(decisions[1].reason.contains("not known yet"));
    assert_eq!(decisions[2].session_id, "");
}

#[test]
fn an_inherited_model_written_as_an_alias_is_still_inherited() {
    let (hook, sink) = new_hook(TrafficMode::Enforce);
    run(&hook, &main_turn("s1", "claude-haiku-4-5-20251001", 100));
    let first = first_turn(
        "s1",
        "a1",
        "claude-haiku-4-5",
        READ_ONLY,
        SEARCH_PROMPT,
        101,
    );
    // Inherited, so it is classified (not kept by the explicit-choice guard) and moves to the
    // cheaper Haiku 5.5.
    assert_eq!(run(&hook, &first).body["model"], HAIKU);
    let decision = &sink.decisions()[0];
    assert_eq!(
        (decision.layer.as_str(), decision.outcome.as_str()),
        ("tools", "applied")
    );
}

#[test]
fn routing_only_downgrades() {
    let (hook, sink) = new_hook(TrafficMode::Enforce);
    run(&hook, &main_turn("s1", HAIKU, 100));
    let cheap = first_turn("s1", "a1", HAIKU, READ_ONLY, SEARCH_PROMPT, 101);
    assert_eq!(run(&hook, &cheap).body["model"], HAIKU);
    let decision = &sink.decisions()[0];
    assert_eq!(decision.outcome, "kept");
    assert!(
        decision
            .reason
            .contains("claude-haiku-5-5 is not cheaper than claude-haiku-5-5")
    );

    // Implementation goes to the mid tier, which costs as much as Sonnet 5.5 or more.
    run(&hook, &main_turn("s2", SONNET, 100));
    let implement = first_turn(
        "s2",
        "a2",
        SONNET,
        &["Read", "Edit", "Bash"],
        "Implement pagination on the users list endpoint please",
        101,
    );
    assert_eq!(run(&hook, &implement).body["model"], SONNET);
    let decision = &sink.decisions()[1];
    assert_eq!(decision.task_class, "implementation");
    assert!(
        decision.reason.contains("not cheaper"),
        "{}",
        decision.reason
    );

    // From Opus 5.5 the mid tier pays.
    run(&hook, &main_turn("s3", OPUS, 100));
    let implement = first_turn(
        "s3",
        "a3",
        OPUS,
        &["Read", "Edit", "Bash"],
        "Implement pagination on the users list endpoint please",
        101,
    );
    assert_eq!(run(&hook, &implement).body["model"], SONNET);
    assert_eq!(detail(&sink.decisions()[2])["tier"], "mid");
}

#[test]
fn allowed_models_limit_the_targets() {
    let (hook, sink) = new_hook_with(RoutingConfig {
        allowed_models: vec!["claude-haiku-4-5-20251001".to_owned()],
        ..config(TrafficMode::Enforce)
    });
    run(&hook, &main_turn("s1", SONNET, 100));
    run(
        &hook,
        &first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101),
    );
    let decision = &sink.decisions()[0];
    assert_eq!(decision.outcome, "kept");
    assert!(
        decision
            .reason
            .contains("claude-haiku-5-5 is not in allowed_models")
    );

    let (hook, sink) = new_hook_with(RoutingConfig {
        allowed_models: vec![HAIKU.to_owned()],
        ..config(TrafficMode::Enforce)
    });
    run(&hook, &main_turn("s1", SONNET, 100));
    let done = run(
        &hook,
        &first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101),
    );
    assert_eq!(done.body["model"], HAIKU);
    assert_eq!(sink.decisions()[0].outcome, "applied");
}

#[test]
fn an_incompatible_request_keeps_its_model() {
    let (hook, sink) = new_hook(TrafficMode::Enforce);
    run(&hook, &main_turn("s1", SONNET, 100));
    let mut first = first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101);
    first.body["thinking"] = json!({"type": "between_tools"});
    assert_eq!(run(&hook, &first).body, first.body);
    let decision = &sink.decisions()[0];
    assert_eq!(detail(decision)["guard"], "target");
    assert!(
        decision
            .reason
            .contains("claude-haiku-5-5 lacks between_tools_at_high"),
        "{}",
        decision.reason
    );

    let mut beta = first_turn("s1", "a2", SONNET, READ_ONLY, SEARCH_PROMPT, 102);
    beta.meta.betas = vec!["brand-new-2027-01-01".to_owned()];
    assert_eq!(run(&hook, &beta).body, beta.body);
    assert!(
        sink.decisions()[1]
            .reason
            .contains("not in the model catalog")
    );
}

#[test]
fn guards_after_classification() {
    // Low confidence: a search prompt for an agent that can edit and run commands.
    let (hook, sink) = new_hook(TrafficMode::Enforce);
    run(&hook, &main_turn("s1", SONNET, 100));
    let risky = first_turn(
        "s1",
        "a1",
        SONNET,
        &["Read", "Edit", "Bash"],
        "Find every place that calls the old pricing function",
        101,
    );
    assert_eq!(run(&hook, &risky).body["model"], SONNET);
    // A planning task is kept by the default tiers.
    let plan = first_turn(
        "s1",
        "a2",
        SONNET,
        READ_ONLY,
        "Planeje a migração do banco de dados para o Postgres 16",
        102,
    );
    run(&hook, &plan);
    // The Explore agent type decides even with tools that change things.
    let mut explore = first_turn("s1", "a3", SONNET, &["Bash"], "do it", 103);
    explore.meta.agent_type = Some("Explore".to_owned());
    assert_eq!(run(&hook, &explore).body["model"], HAIKU);
    // No signal at all.
    run(&hook, &first_turn("s1", "a4", SONNET, &["Edit"], "ok", 104));

    let decisions = sink.decisions();
    let guards: Vec<Value> = decisions
        .iter()
        .map(|d| detail(d)["guard"].clone())
        .collect();
    assert_eq!(
        guards,
        [
            json!("confidence"),
            json!("tier"),
            Value::Null,
            json!("confidence")
        ]
    );
    assert!(
        decisions[0]
            .reason
            .contains("confidence 0.60 below the minimum 0.80")
    );
    assert!(
        decisions[1]
            .reason
            .contains("planning is configured to keep")
    );
    assert_eq!(decisions[2].layer, "headers");
    assert_eq!(decisions[3].layer, "default");
}

#[test]
fn the_confidence_threshold_is_inclusive() {
    for (min, routed) in [(0.85, true), (0.86, false)] {
        let (hook, sink) = new_hook_with(RoutingConfig {
            min_confidence: min,
            ..config(TrafficMode::Enforce)
        });
        run(&hook, &main_turn("s1", SONNET, 100));
        run(
            &hook,
            &first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101),
        );
        assert_eq!(sink.decisions()[0].outcome == "applied", routed, "{min}");
    }
}

#[test]
fn a_small_saving_is_not_worth_it() {
    let (hook, sink) = new_hook_with(RoutingConfig {
        min_saving_usd: 1.0,
        ..config(TrafficMode::Enforce)
    });
    run(&hook, &main_turn("s1", SONNET, 100));
    run(
        &hook,
        &first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101),
    );
    let decision = &sink.decisions()[0];
    let detail = detail(decision);
    assert_eq!(detail["guard"], "saving");
    assert_eq!(detail["candidate"], HAIKU);
    assert!(detail["candidate_est_cost"].as_f64().unwrap() > 0.0);
    assert!((decision.est_cost_to - decision.est_cost_from).abs() < 1e-12);
}

#[test]
fn unknown_and_unpriced_models_are_never_routed() {
    let (hook, sink) = new_hook(TrafficMode::Enforce);
    run(&hook, &main_turn("s1", "claude-3-haiku", 100));
    let old = first_turn("s1", "a1", "claude-3-haiku", READ_ONLY, SEARCH_PROMPT, 101);
    assert_eq!(run(&hook, &old).body, old.body);
    let mut no_model = first_turn("s1", "a2", SONNET, READ_ONLY, SEARCH_PROMPT, 102);
    no_model.body.as_object_mut().unwrap().remove("model");
    assert_eq!(run(&hook, &no_model).body, no_model.body);
    let aux = title("s1", "claude-3-haiku", 4000, 103);
    assert_eq!(run(&hook, &aux).body, aux.body);
    let decisions = sink.decisions();
    assert!(
        decisions[0]
            .reason
            .contains("\"claude-3-haiku\" is not in the model catalog")
    );
    assert!(
        decisions[1]
            .reason
            .contains("\"\" is not in the model catalog")
    );
    assert_eq!(decisions[2].scope, "request");
    assert!(
        decisions
            .iter()
            .all(|d| d.outcome == "kept" && d.layer == "default")
    );

    // A catalog model missing from the price table.
    let sink = std::sync::Arc::new(Sink::default());
    let hook = RoutingHook::new(
        config(TrafficMode::Enforce),
        tokred_core::pricing::PriceTable::default(),
        sink.clone(),
    );
    run(&hook, &main_turn("s1", SONNET, 100));
    run(
        &hook,
        &first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101),
    );
    let decision = &sink.decisions()[0];
    assert!(decision.reason.contains("no price for claude-sonnet-5-5"));
    assert!(decision.est_cost_from.abs() < 1e-12);
}

#[test]
fn an_empty_tier_keeps_the_model() {
    let mut catalog = Catalog::embedded();
    catalog.tiers.remove("cheap");
    let (hook, sink) = new_hook(TrafficMode::Enforce);
    let hook = hook.with_catalog(catalog);
    run(&hook, &main_turn("s1", SONNET, 100));
    run(
        &hook,
        &first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101),
    );
    assert!(
        sink.decisions()[0]
            .reason
            .contains("no model in the cheap tier")
    );
}

#[test]
fn a_rejected_rewrite_stops_the_agent_for_good() {
    let (hook, sink) = new_hook(TrafficMode::Enforce);
    run(&hook, &main_turn("s1", SONNET, 100));
    let first = first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101);
    let done = run(&hook, &first);
    reject(&hook, &first, &done, 400);
    let decisions = sink.decisions();
    assert_eq!(decisions.len(), 2);
    let stopped = &decisions[1];
    assert_eq!(stopped.id, decisions[0].id, "the same decision, updated");
    assert_eq!(stopped.outcome, "fallback");
    assert!(
        stopped
            .reason
            .contains("rejected the rewritten request with status 400")
    );
    assert_eq!(detail(stopped)["original_reads_target_thinking"], true);
    // From now on the agent keeps its model.
    let later_req = later_turn(&first, 102);
    let later = run(&hook, &later_req);
    assert_eq!(later.body, later_req.body);
    // A second report for the same agent records nothing new.
    reject(&hook, &first, &done, 400);
    assert_eq!(sink.decisions().len(), 2);
    // A request that was not rewritten is not ours to count.
    reject(&hook, &later_req, &later, 400);
    assert_eq!(hook.status(Timestamp(102)).breaker.failures, 2);
}

#[test]
fn a_sticky_rewrite_stops_when_a_later_request_cannot_follow() {
    let (hook, sink) = new_hook(TrafficMode::Enforce);
    run(&hook, &main_turn("s1", SONNET, 100));
    // A later request gains a beta the target lacks.
    let first = first_turn("s1", "a1", SONNET, READ_ONLY, SEARCH_PROMPT, 101);
    run(&hook, &first);
    let mut later = later_turn(&first, 102);
    later.meta.betas = vec!["thinking-display-updates-2026-08-18".to_owned()];
    assert_eq!(run(&hook, &later).body, later.body);
    let stopped = &sink.decisions()[1];
    assert_eq!(stopped.outcome, "fallback");
    assert!(stopped.reason.contains("lacks thinking_display_updates"));

    // The requested model changes mid-run.
    let first = first_turn("s1", "a2", SONNET, READ_ONLY, SEARCH_PROMPT, 103);
    run(&hook, &first);
    let mut later = later_turn(&first, 104);
    later.body["model"] = json!(OPUS);
    assert_eq!(run(&hook, &later).body["model"], OPUS);
    assert!(sink.decisions()[3].reason.contains("changed mid-run"));

    // A catalog that no longer knows the models.
    let first = first_turn("s1", "a3", SONNET, READ_ONLY, SEARCH_PROMPT, 105);
    let (hook2, sink2) = new_hook(TrafficMode::Enforce);
    run(&hook2, &main_turn("s1", SONNET, 100));
    run(&hook2, &first);
    let hook2 = hook2.with_catalog(Catalog::default());
    let later = later_turn(&first, 106);
    assert_eq!(run(&hook2, &later).body, later.body);
    let stopped = &sink2.decisions()[1];
    assert!(stopped.reason.contains("left the catalog"));
    // Without catalog data the thinking readability is unknown, so reported as false.
    assert_eq!(detail(stopped)["original_reads_target_thinking"], false);
}

#[test]
fn the_circuit_breaker_opens_and_closes() {
    let (hook, sink) = new_hook_with(RoutingConfig {
        cooldown: "10m".to_owned(),
        ..config(TrafficMode::Enforce)
    });
    run(&hook, &main_turn("s1", SONNET, 100));
    // An agent that is being rewritten without trouble.
    let healthy = first_turn("s1", "healthy", SONNET, READ_ONLY, SEARCH_PROMPT, 100);
    assert_eq!(run(&hook, &healthy).body["model"], HAIKU);
    // Three rejected rewrites out of 50 is above 5%.
    for n in 0..3 {
        let req = first_turn(
            "s1",
            &format!("bad{n}"),
            SONNET,
            READ_ONLY,
            SEARCH_PROMPT,
            101,
        );
        let done = run(&hook, &req);
        assert_eq!(done.body["model"], HAIKU);
        reject(&hook, &req, &done, 400);
    }
    let status = hook.status(Timestamp(102));
    assert!(status.breaker.open);
    assert_eq!(status.breaker.open_until, Some(101 + 600));

    // While open: new agents are kept, sticky rewrites stop.
    let new = first_turn("s1", "new", SONNET, READ_ONLY, SEARCH_PROMPT, 200);
    assert_eq!(run(&hook, &new).body, new.body);
    let kept = sink.decisions().last().cloned().unwrap();
    assert_eq!(kept.outcome, "kept");
    assert_eq!(kept.model_to, SONNET);
    assert!(kept.reason.contains("circuit breaker is open"));
    assert_eq!(detail(&kept)["guard"], "breaker");
    let later_req = later_turn(&healthy, 201);
    assert_eq!(run(&hook, &later_req).body, later_req.body);
    let stopped = sink.decisions().last().cloned().unwrap();
    assert_eq!(
        (stopped.outcome.as_str(), stopped.agent_id.as_deref()),
        ("fallback", Some("healthy"))
    );

    // Auxiliary requests are kept too.
    run(&hook, &main_turn("s9", OPUS, 300));
    let aux = title("s9", OPUS, 8000, 300);
    assert_eq!(run(&hook, &aux).body, aux.body);

    // After the cooldown, routing resumes with a fresh window.
    let after = first_turn("s1", "after", SONNET, READ_ONLY, SEARCH_PROMPT, 701);
    assert_eq!(run(&hook, &after).body["model"], HAIKU);
    let status = hook.status(Timestamp(701));
    assert!(!status.breaker.open);
    assert_eq!((status.breaker.rewrites, status.breaker.failures), (1, 0));
}

#[test]
fn auxiliary_requests_move_only_when_it_pays() {
    let (hook, sink) = new_hook(TrafficMode::Enforce);
    run(&hook, &main_turn("s1", OPUS, 100));
    // A session title on Opus 5.5 with a few thousand tokens of context.
    let big = title("s1", OPUS, 8000, 101);
    let done = run(&hook, &big);
    assert!(done.changed);
    assert_eq!(done.body["model"], HAIKU);
    let decision = &sink.decisions()[0];
    assert_eq!(
        (
            decision.scope.as_str(),
            decision.task_class.as_str(),
            decision.layer.as_str()
        ),
        ("request", "auxiliary", "shape")
    );
    assert!(decision.agent_id.is_none());
    // A tiny one on Sonnet 5.5 saves less than min_saving_usd.
    let small = title("s1", SONNET, 400, 102);
    assert_eq!(run(&hook, &small).body, small.body);
    assert_eq!(detail(&sink.decisions()[1])["guard"], "saving");
    // Cache markers mean the request writes or reads the prompt cache.
    let mut cached = title("s1", OPUS, 8000, 103);
    cached.body["system"] =
        json!([{"type": "text", "text": "t", "cache_control": {"type": "ephemeral"}}]);
    assert_eq!(run(&hook, &cached).body, cached.body);
    let mut auto_cached = title("s1", OPUS, 8000, 104);
    auto_cached.body["cache_control"] = json!({"type": "ephemeral"});
    assert_eq!(run(&hook, &auto_cached).body, auto_cached.body);
    // A header-classified auxiliary request with tools may share the main loop's cache.
    let mut with_tools = main_turn("s1", OPUS, 105);
    with_tools.meta.request_class = Some("auxiliary".to_owned());
    assert_eq!(run(&hook, &with_tools).body, with_tools.body);
    let decisions = sink.decisions();
    assert!(
        decisions[2..5]
            .iter()
            .all(|d| detail(d)["guard"] == "cache")
    );
    assert_eq!(decisions[4].layer, "headers");
    assert!(
        decisions[4]
            .reason
            .contains("may share the main loop's prompt cache")
    );

    // A rejected auxiliary rewrite is recorded against its decision and counted.
    reject(&hook, &big, &done, 400);
    let last = sink.decisions().last().cloned().unwrap();
    assert_eq!(
        (last.id.as_str(), last.outcome.as_str()),
        (decision.id.as_str(), "fallback")
    );
    assert_eq!(hook.status(Timestamp(106)).breaker.failures, 1);
    // An unknown decision id records nothing.
    let mut forgotten = done.clone();
    forgotten.notes.decision_id = Some("nope".to_owned());
    reject(&hook, &big, &forgotten, 400);
    assert_eq!(sink.decisions().len(), 6);
}

#[test]
fn main_loop_and_compaction_requests_are_never_touched() {
    for mode in [TrafficMode::Off, TrafficMode::Observe, TrafficMode::Enforce] {
        let (hook, sink) = new_hook(mode);
        let main = main_turn("s1", OPUS, 100);
        let done = run(&hook, &main);
        assert_eq!(done.body, main.body);
        assert!(!done.changed);
        let mut compaction = main_turn("s1", OPUS, 101);
        compaction.meta.compaction = Some("auto".to_owned());
        let done = run(&hook, &compaction);
        assert_eq!(done.body, compaction.body);
        assert!(sink.decisions().is_empty());
    }
}

#[test]
fn decision_ids_are_unique() {
    let (hook, sink) = new_hook(TrafficMode::Observe);
    run(&hook, &main_turn("s1", SONNET, 100));
    for n in 0..20 {
        run(
            &hook,
            &first_turn(
                "s1",
                &format!("a{n}"),
                SONNET,
                READ_ONLY,
                SEARCH_PROMPT,
                101,
            ),
        );
    }
    let mut ids: Vec<String> = sink.decisions().into_iter().map(|d| d.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 20);
}
