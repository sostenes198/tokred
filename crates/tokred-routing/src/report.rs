//! Text for `tokred routing explain` and `tokred routing status` (pure).

use std::fmt::Write as _;

use tokred_core::routing::RoutingDecision;

use crate::hook::RoutingStatus;

/// A multi-line explanation of one decision: who, the classification layer and confidence,
/// the reason, the models, the outcome and the cost estimate.
#[must_use]
pub fn explain(decision: &RoutingDecision) -> String {
    let mut out = format!("decision {} at {}\n", decision.id, decision.ts.0);
    let who = match &decision.agent_id {
        Some(agent) => format!("subagent {agent}"),
        None => "one request".to_owned(),
    };
    let _ = writeln!(
        out,
        "  scope:     {} ({who}) in session {}",
        decision.scope,
        display_or_unknown(&decision.session_id)
    );
    let _ = writeln!(
        out,
        "  class:     {} (confidence {:.2}, layer {})",
        decision.task_class, decision.confidence, decision.layer
    );
    let _ = writeln!(out, "  reason:    {}", decision.reason);
    let _ = writeln!(
        out,
        "  model:     {} -> {}",
        display_or_unknown(&decision.model_from),
        display_or_unknown(&decision.model_to)
    );
    let _ = writeln!(
        out,
        "  outcome:   {} (mode {})",
        decision.outcome, decision.mode
    );
    let saving = decision.est_cost_from - decision.est_cost_to;
    let _ = write!(
        out,
        "  estimate:  ${:.6} -> ${:.6}",
        decision.est_cost_from, decision.est_cost_to
    );
    if saving > 0.0 {
        let _ = write!(out, " (saving ${saving:.6} on this request)");
    }
    let _ = writeln!(out);
    if !matches!(decision.detail.as_str(), "" | "{}") {
        let _ = writeln!(out, "  detail:    {}", decision.detail);
    }
    let _ = writeln!(out, "  rules:     {}", decision.rules_version);
    out
}

fn display_or_unknown(value: &str) -> &str {
    if value.is_empty() { "(unknown)" } else { value }
}

/// The state of a running proxy's routing: mode, circuit breaker and what it remembers.
#[must_use]
pub fn render_status(status: &RoutingStatus) -> String {
    let breaker = &status.breaker;
    let state = match (breaker.open, breaker.open_until) {
        (true, Some(until)) => format!("open until {until}"),
        _ => "closed".to_owned(),
    };
    let mut out = format!(
        "routing: mode {}, rules {}\n",
        status.mode, status.rules_version
    );
    let _ = writeln!(
        out,
        "circuit breaker: {state} ({} of the last {} rewrites rejected, {:.1}%; limit {:.1}%, \
         cooldown {}s)",
        breaker.failures,
        breaker.rewrites,
        breaker.failure_rate * 100.0,
        breaker.max_failure_rate * 100.0,
        breaker.cooldown_secs
    );
    let _ = writeln!(
        out,
        "remembered: {} subagents, {} sessions",
        status.tracked_agents, status.tracked_sessions
    );
    out
}

/// One line summarizing decisions (e.g. the latest ones from the store): how many per outcome
/// and the estimated saving of the routed ones (hypothetical for `observed`).
#[must_use]
pub fn summarize_decisions(decisions: &[RoutingDecision]) -> String {
    let count = |outcome: &str| decisions.iter().filter(|d| d.outcome == outcome).count();
    // `+ 0.0` turns the `-0.0` that an empty float sum yields into `0.0` (it would print `$-0.000000`).
    let saving = decisions
        .iter()
        .filter(|d| matches!(d.outcome.as_str(), "applied" | "observed"))
        .map(|d| d.est_cost_from - d.est_cost_to)
        .sum::<f64>()
        + 0.0;
    format!(
        "{} decisions: {} applied, {} observed, {} kept, {} fallback; estimated saving ${saving:.6}",
        decisions.len(),
        count("applied"),
        count("observed"),
        count("kept"),
        count("fallback")
    )
}

/// What the decisions of a period say, to judge whether turning routing on is worth it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PeriodSummary {
    /// Decisions taken in the period.
    pub decisions: usize,
    /// Requests routing changed (`applied`).
    pub applied: usize,
    /// Requests routing would have changed (`observed`).
    pub observed: usize,
    /// Requests it left alone.
    pub kept: usize,
    /// Requests that were rewritten, rejected and sent as asked.
    pub fallback: usize,
    /// Estimated saving of the applied and observed decisions, USD.
    pub saving_usd: f64,
    /// Applied and observed decisions per task class, with their saving, the largest saving first.
    pub by_task: Vec<(String, usize, f64)>,
}

/// Counts the decisions of a period by outcome and adds up what the rerouted ones would save
/// (`applied` and `observed`; the estimate is made on the request as it was sent).
#[must_use]
pub fn summarize_period(decisions: &[RoutingDecision]) -> PeriodSummary {
    let count = |outcome: &str| decisions.iter().filter(|d| d.outcome == outcome).count();
    let mut by_task: Vec<(String, usize, f64)> = Vec::new();
    for decision in decisions
        .iter()
        .filter(|d| matches!(d.outcome.as_str(), "applied" | "observed"))
    {
        let saving = decision.est_cost_from - decision.est_cost_to;
        match by_task
            .iter_mut()
            .find(|(task, _, _)| *task == decision.task_class)
        {
            Some((_, requests, total)) => {
                *requests += 1;
                *total += saving;
            }
            None => by_task.push((decision.task_class.clone(), 1, saving)),
        }
    }
    by_task.sort_by(|a, b| b.2.total_cmp(&a.2));
    // `+ 0.0` turns the `-0.0` that an empty float sum yields into `0.0`.
    let saving_usd = by_task.iter().map(|(_, _, saving)| saving).sum::<f64>() + 0.0;
    PeriodSummary {
        decisions: decisions.len(),
        applied: count("applied"),
        observed: count("observed"),
        kept: count("kept"),
        fallback: count("fallback"),
        saving_usd,
        by_task,
    }
}

/// One paragraph that says whether routing is worth turning on, from the period's decisions.
/// `label` names the period ("last 7 days") and `days` is its length, to project a month.
#[must_use]
pub fn verdict(summary: &PeriodSummary, mode: &str, label: &str, days: f64) -> String {
    let month = summary.saving_usd * 30.0 / days.max(1.0 / 24.0);
    if mode == "off" {
        return "Routing is off: nothing is recorded. Turn recording on with `tokred routing off` (it keeps changing nothing), or routing itself with `tokred routing on`.".to_owned();
    }
    if summary.decisions == 0 {
        return format!(
            "No routing decisions in the {label}: use `tokred claude` and run some subagents."
        );
    }
    if mode == "enforce" {
        return format!(
            "Routing changed {} of {} requests in the {label} and saved about ${:.2} (about ${month:.2} a month at this pace).",
            summary.applied, summary.decisions, summary.saving_usd
        );
    }
    if summary.observed == 0 {
        return format!(
            "Of {} requests in the {label}, none would have been rerouted (all kept): turning routing on would change nothing yet.",
            summary.decisions
        );
    }
    format!(
        "Turning routing on would have changed {} of {} requests in the {label} and saved about ${:.2} (about ${month:.2} a month at this pace). To turn it on: tokred routing on",
        summary.observed, summary.decisions, summary.saving_usd
    )
}

#[cfg(test)]
mod tests {
    use tokred_core::Timestamp;

    use super::*;
    use crate::breaker::BreakerStatus;

    fn decision(outcome: &str) -> RoutingDecision {
        RoutingDecision {
            id: "rd-1".to_owned(),
            ts: Timestamp(1_700_000_000),
            session_id: "s1".to_owned(),
            agent_id: Some("a1".to_owned()),
            scope: "agent".to_owned(),
            task_class: "search".to_owned(),
            confidence: 0.85,
            layer: "tools".to_owned(),
            reason: "read-only tools only (Read)".to_owned(),
            model_from: "claude-sonnet-5-5".to_owned(),
            model_to: "claude-haiku-5-5".to_owned(),
            mode: "enforce".to_owned(),
            est_cost_from: 0.01,
            est_cost_to: 0.001,
            rules_version: "r".to_owned(),
            outcome: outcome.to_owned(),
            detail: r#"{"tier":"cheap"}"#.to_owned(),
        }
    }

    #[test]
    fn explain_shows_every_field() {
        let text = explain(&decision("applied"));
        for expected in [
            "decision rd-1 at 1700000000",
            "agent (subagent a1) in session s1",
            "search (confidence 0.85, layer tools)",
            "claude-sonnet-5-5 -> claude-haiku-5-5",
            "applied (mode enforce)",
            "$0.010000 -> $0.001000 (saving $0.009000",
            r#"detail:    {"tier":"cheap"}"#,
            "rules:     r",
        ] {
            assert!(text.contains(expected), "{expected:?} in\n{text}");
        }
        let kept = RoutingDecision {
            agent_id: None,
            session_id: String::new(),
            model_to: String::new(),
            est_cost_to: 0.01,
            detail: "{}".to_owned(),
            ..decision("kept")
        };
        let text = explain(&kept);
        assert!(text.contains("(one request) in session (unknown)"));
        assert!(text.contains("-> (unknown)"));
        assert!(!text.contains("saving") && !text.contains("detail"));
    }

    #[test]
    fn status_and_summary_render() {
        let mut status = RoutingStatus {
            mode: "enforce".to_owned(),
            rules_version: "r".to_owned(),
            breaker: BreakerStatus {
                open: false,
                open_until: None,
                rewrites: 10,
                failures: 1,
                failure_rate: 0.02,
                max_failure_rate: 0.05,
                cooldown_secs: 1800,
            },
            tracked_agents: 3,
            tracked_sessions: 2,
        };
        let text = render_status(&status);
        assert!(text.contains("mode enforce"));
        assert!(text.contains("closed (1 of the last 10 rewrites rejected, 2.0%; limit 5.0%"));
        assert!(text.contains("3 subagents, 2 sessions"));
        status.breaker.open = true;
        status.breaker.open_until = Some(99);
        assert!(render_status(&status).contains("open until 99"));

        let summary = summarize_decisions(&[
            decision("applied"),
            decision("observed"),
            decision("kept"),
            decision("fallback"),
        ]);
        assert_eq!(
            summary,
            "4 decisions: 1 applied, 1 observed, 1 kept, 1 fallback; estimated saving $0.018000"
        );
    }

    #[test]
    fn nothing_saved_prints_a_plain_zero() {
        let summary = summarize_decisions(&[decision("kept")]);
        assert!(summary.ends_with("estimated saving $0.000000"), "{summary}");
    }

    fn rerouted(task: &str, outcome: &str, from: f64, to: f64) -> RoutingDecision {
        RoutingDecision {
            task_class: task.to_owned(),
            est_cost_from: from,
            est_cost_to: to,
            ..decision(outcome)
        }
    }

    #[test]
    fn a_period_counts_the_outcomes_and_adds_up_the_rerouted_saving() {
        let decisions = [
            rerouted("search", "observed", 0.10, 0.02),
            rerouted("search", "observed", 0.10, 0.04),
            rerouted("summary", "applied", 0.50, 0.10),
            rerouted("other", "kept", 0.30, 0.30),
            rerouted("other", "fallback", 0.30, 0.30),
        ];
        let summary = summarize_period(&decisions);
        assert_eq!(
            (
                summary.decisions,
                summary.applied,
                summary.observed,
                summary.kept,
                summary.fallback
            ),
            (5, 1, 2, 1, 1)
        );
        assert!((summary.saving_usd - 0.54).abs() < 1e-9, "{summary:?}");
        let tasks: Vec<(&str, usize)> = summary
            .by_task
            .iter()
            .map(|(task, requests, _)| (task.as_str(), *requests))
            .collect();
        assert_eq!(
            tasks,
            [("summary", 1), ("search", 2)],
            "largest saving first"
        );
        assert_eq!(summarize_period(&[]), PeriodSummary::default());
    }

    #[test]
    fn the_verdict_says_whether_routing_is_worth_turning_on() {
        let none = PeriodSummary::default();
        assert!(verdict(&none, "off", "last 7 days", 7.0).contains("Routing is off"));
        assert!(verdict(&none, "observe", "last 7 days", 7.0).contains("No routing decisions"));

        let all_kept = summarize_period(&[decision("kept"), decision("kept")]);
        let text = verdict(&all_kept, "observe", "last 7 days", 7.0);
        assert!(text.contains("none would have been rerouted"), "{text}");

        let worth =
            summarize_period(&[rerouted("search", "observed", 0.10, 0.02), decision("kept")]);
        let text = verdict(&worth, "observe", "last 7 days", 7.0);
        assert!(
            text.contains("would have changed 1 of 2 requests")
                && text.contains("saved about $0.08")
                && text.contains("about $0.34 a month")
                && text.contains("tokred routing on"),
            "{text}"
        );

        let applied = summarize_period(&[rerouted("search", "applied", 0.10, 0.02)]);
        let text = verdict(&applied, "enforce", "last 24 hours", 1.0);
        assert!(text.contains("Routing changed 1 of 1 requests"), "{text}");
    }
}
