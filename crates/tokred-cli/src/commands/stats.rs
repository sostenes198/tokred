//! `tokred stats` and `tokred stats latency`.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::json;
use tokred_metrics::{GroupBy, Stats, StatsInput, compute_stats, latency_stats, tradeoff};

use super::data::{Loaded, load};
use crate::args::{LatencyArgs, StatsArgs, StatsBy};
use crate::env::Env;
use crate::io::Io;
use crate::term::Term;
use crate::view::{render_latency, render_stats};

/// Stats of the loaded period, grouped as asked.
pub fn compute(loaded: &Loaded, group_by: GroupBy) -> Stats {
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
    }
}

pub fn run(env: &Env, io: &mut Io, dir_flag: Option<PathBuf>, args: &StatsArgs) -> Result<()> {
    let loaded = load(env, io, dir_flag, &args.since)?;
    let stats = compute(&loaded, group_by(args.by));
    if args.json {
        let text =
            serde_json::to_string_pretty(&json!({ "period": loaded.label, "stats": stats }))?;
        writeln!(io.out(), "{text}")?;
    } else {
        let shown = render_stats(Term::new(io.color()), &stats, &loaded.label);
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
    let latency = latency_stats(&loaded.events, loaded.spawn_overhead_us, loaded.range);
    let stats = compute(&loaded, GroupBy::Model);
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
}
