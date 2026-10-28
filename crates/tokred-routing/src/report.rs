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
}
