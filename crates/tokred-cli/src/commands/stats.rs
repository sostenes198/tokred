//! `tokred stats` and `tokred stats latency`.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::json;
use tokred_metrics::{GroupBy, Stats, StatsInput, compute_stats, latency_stats, tradeoff};

use super::data::{Gathered, load};
use crate::args::{LatencyArgs, StatsArgs, StatsBy};
use crate::env::Env;
use crate::io::Io;
use crate::term::Term;
use crate::view::{render_latency, render_proxy, render_stats};

/// Stats of the loaded period, grouped as asked.
pub fn compute(loaded: &Gathered, group_by: GroupBy) -> Stats {
    compute_stats(&StatsInput {
        usage: &loaded.usage,
        prices: &loaded.prices,
        events: &loaded.events,
        range: loaded.range,
        group_by,
        now: loaded.now,
        first_configure: loaded.first_configure,
    })
}

fn group_by(by: Option<StatsBy>) -> GroupBy {
    match by {
        None | Some(StatsBy::Model) => GroupBy::Model,
        Some(StatsBy::Day) => GroupBy::Day,
        Some(StatsBy::Project) => GroupBy::Project,
        Some(StatsBy::Origin) => GroupBy::Origin,
        Some(StatsBy::AgentType) => GroupBy::AgentType,
    }
}

pub fn run(env: &Env, io: &mut Io, dir_flag: Option<PathBuf>, args: &StatsArgs) -> Result<()> {
    let loaded = load(env, io, dir_flag, &args.since)?;
    let data = &loaded.data;
    let stats = compute(data, group_by(args.by));
    let proxy = data.proxy();
    if args.json {
        let mut value = json!({ "period": data.label, "stats": stats });
        if let Some(proxy) = &proxy {
            value["proxy"] = serde_json::to_value(proxy)?;
        }
        writeln!(io.out(), "{}", serde_json::to_string_pretty(&value)?)?;
    } else {
        let term = Term::new(io.color());
        let mut shown = render_stats(term, &stats, &data.label);
        if let Some(proxy) = &proxy {
            shown.push_str(&render_proxy(term, proxy));
        }
        write!(io.out(), "{shown}")?;
    }
    Ok(())
}

pub fn run_latency(
    env: &Env,
    io: &mut Io,
    dir_flag: Option<PathBuf>,
    args: &LatencyArgs,
) -> Result<()> {
    let loaded = load(env, io, dir_flag, &args.since)?;
    let loaded = &loaded.data;
    let latency = latency_stats(&loaded.events, loaded.spawn_overhead_us, loaded.range);
    let stats = compute(loaded, GroupBy::Model);
    let trade = tradeoff(&latency, &stats);
    if args.json {
        let text = serde_json::to_string_pretty(&json!({
            "period": loaded.label,
            "latency": latency,
            "tradeoff": trade,
        }))?;
        writeln!(io.out(), "{text}")?;
    } else {
        let shown = render_latency(
            Term::new(io.color()),
            &latency,
            &trade,
            args.by,
            &loaded.label,
        );
        write!(io.out(), "{shown}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted;

    fn stats_args(by: Option<StatsBy>, json: bool) -> StatsArgs {
        StatsArgs {
            since: "7d".to_owned(),
            by,
            json,
        }
    }

    fn run_stats(sb: &Sandbox, args: &StatsArgs) -> (Result<()>, String) {
        let (mut io, out) = scripted(false, "");
        let result = run(&sb.env, &mut io, Some(sb.claude.clone()), args);
        (result, out.text())
    }

    fn seeded() -> Sandbox {
        let sb = Sandbox::new();
        sb.configure();
        sb.seed_usage();
        sb.seed_events();
        sb
    }

    #[test]
    fn the_dashboard_shows_the_seeded_usage() {
        let sb = seeded();
        let (result, out) = run_stats(&sb, &stats_args(None, false));
        result.unwrap();
        for expected in [
            "tokred Token Savings (last 7 days)",
            "Messages:",
            "(1 sessions)",
            "Efficiency meter:",
            "By Model",
            "claude-opus-4-1-20250805",
            "claude-sonnet-4-20250514",
        ] {
            assert!(out.contains(expected), "missing {expected:?} in:\n{out}");
        }
        assert!(
            out.contains("estimate: 1 blocks") || out.contains("estimate: 2 blocks"),
            "{out}"
        );
    }

    #[test]
    fn grouping_changes_the_table() {
        let sb = seeded();
        let (_, by_day) = run_stats(&sb, &stats_args(Some(StatsBy::Day), false));
        assert!(by_day.contains("By Day"));
        let (_, by_project) = run_stats(&sb, &stats_args(Some(StatsBy::Project), false));
        assert!(
            by_project.contains("By Project") && by_project.contains("-tmp-proj"),
            "{by_project}"
        );
        let (_, by_model) = run_stats(&sb, &stats_args(Some(StatsBy::Model), false));
        assert!(by_model.contains("By Model"));
    }

    #[test]
    fn json_wraps_the_stats_with_the_period() {
        let sb = seeded();
        let (result, out) = run_stats(&sb, &stats_args(None, true));
        result.unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["period"], "last 7 days");
        assert_eq!(value["stats"]["totals"]["messages"], 2);
        assert_eq!(value["stats"]["savings"]["guard"]["estimate"], true);
    }

    #[test]
    fn an_empty_period_is_explained() {
        let sb = Sandbox::new();
        sb.configure();
        let (result, out) = run_stats(&sb, &stats_args(None, false));
        result.unwrap();
        assert!(out.contains("No Claude Code usage found for this period."));
    }

    #[test]
    fn latency_is_shown_as_a_table_or_json() {
        let sb = seeded();
        let args = LatencyArgs {
            since: "7d".to_owned(),
            by: None,
            json: false,
        };
        let (mut io, out) = scripted(false, "");
        run_latency(&sb.env, &mut io, Some(sb.claude.clone()), &args).unwrap();
        let text = out.text();
        for expected in [
            "tokred Hook Latency (last 7 days)",
            "Hook runs:",
            "By Hook",
            "guard-read",
            "Trade-off:",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }

        let json_args = LatencyArgs {
            since: "7d".to_owned(),
            by: Some(crate::args::LatencyBy::Day),
            json: true,
        };
        let (mut io, out) = scripted(false, "");
        run_latency(&sb.env, &mut io, Some(sb.claude.clone()), &json_args).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out.text()).unwrap();
        assert!(value["latency"]["runs"].as_u64().unwrap() >= 6);
        assert!(value["tradeoff"]["estimate"].as_bool().unwrap());
    }
    fn seeded_with_proxy() -> Sandbox {
        let sb = seeded();
        sb.seed_proxy();
        sb
    }

    #[test]
    fn without_the_proxy_the_dashboard_and_the_json_are_unchanged() {
        let sb = seeded();
        let (result, out) = run_stats(&sb, &stats_args(None, false));
        result.unwrap();
        for absent in ["Proxy", "Data source", "Model routing", "Cache misses"] {
            assert!(!out.contains(absent), "unexpected {absent:?} in:\n{out}");
        }
        let (_, json) = run_stats(&sb, &stats_args(None, true));
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(value.get("proxy").is_none());
        assert!(
            value["stats"]["groups"]
                .as_array()
                .unwrap()
                .iter()
                .all(|g| g["key"].as_str().unwrap().starts_with("claude-"))
        );
    }

    #[test]
    fn the_proxy_sections_follow_the_dashboard() {
        let sb = seeded_with_proxy();
        let (result, out) = run_stats(&sb, &stats_args(None, false));
        result.unwrap();
        for expected in [
            "By Model",
            "Proxy\n",
            "Data source:  3 messages measured by the proxy, 0 from transcripts only",
            "Requests:     3 handled, 0 failed",
            "Overhead:",
            "Routing:      1 applied (~$0.60 saved) · 1 observed (~$0.40 if applied)",
            "Cache misses: 1 of 1 main-loop messages (100.0%)",
            "Model routing",
            "claude-haiku-4-5 *",
            "Cache misses by cause",
            "ttl_expired",
            "▶ The last 3 prompts were simple; sonnet would have done",
        ] {
            assert!(out.contains(expected), "missing {expected:?} in:\n{out}");
        }
        // The rewritten request is priced as haiku, not as the sonnet the transcript says.
        let by_model = out.split("By Model").nth(1).unwrap();
        let by_model = by_model.split("Proxy").next().unwrap();
        assert!(by_model.contains("claude-haiku-4-5"), "{by_model}");
        assert!(
            !by_model.contains("claude-sonnet-4-20250514 "),
            "{by_model}"
        );
    }

    #[test]
    fn the_json_carries_the_proxy_only_when_it_recorded_something() {
        let sb = seeded_with_proxy();
        let (result, out) = run_stats(&sb, &stats_args(None, true));
        result.unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["stats"]["totals"]["messages"], 3);
        assert_eq!(value["proxy"]["coverage"]["measured"], 3);
        assert_eq!(value["proxy"]["routing"]["applied"], 1);
        assert_eq!(value["proxy"]["overhead"]["over_budget"], false);
        assert_eq!(value["proxy"]["suggestion"]["id"], "sg1");
        assert_eq!(
            value["proxy"]["models"][0]["effective"], "claude-haiku-4-5",
            "most requests first, and the answering model is named"
        );
    }

    #[test]
    fn requests_can_be_grouped_by_origin_and_by_agent_type() {
        let sb = seeded_with_proxy();
        let (result, by_origin) = run_stats(&sb, &stats_args(Some(StatsBy::Origin), false));
        result.unwrap();
        for expected in ["By Origin", "main", "subagent", "auxiliary"] {
            assert!(
                by_origin.contains(expected),
                "missing {expected:?}\n{by_origin}"
            );
        }
        let (_, by_agent) = run_stats(&sb, &stats_args(Some(StatsBy::AgentType), false));
        assert!(
            by_agent.contains("By Agent Type") && by_agent.contains("Explore"),
            "{by_agent}"
        );
        assert!(by_agent.contains("(unknown)"), "{by_agent}");

        let (_, json) = run_stats(&sb, &stats_args(Some(StatsBy::AgentType), true));
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["stats"]["group_by"], "agent_type");

        // Without the proxy the transcripts still say main or subagent.
        let plain = seeded();
        let (_, text) = run_stats(&plain, &stats_args(Some(StatsBy::Origin), false));
        assert!(
            text.contains("By Origin") && text.contains("main"),
            "{text}"
        );
    }
}
