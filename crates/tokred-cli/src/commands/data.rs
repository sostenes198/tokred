//! Loading what `stats` and `metrics` need: ingest the transcripts, then read usage, hook events,
//! prices and the first-configure time from the store.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use tokred_core::Timestamp;
use tokred_core::hook_event::{HookEventRecord, HookEventRepo};
use tokred_core::meta::MetaRepo;
use tokred_core::pricing::PriceTable;
use tokred_core::usage::{UsageRecord, UsageRepo};
use tokred_install::StdFileSystem;
use tokred_metrics::{StdTranscriptFs, TimeRange, ingest, range_from_span};
use tokred_pricing::load_table;
use tokred_store::Store;

use super::{SystemClock, existing_store, select_claude_dir};
use crate::env::Env;
use crate::io::Io;

/// Meta key: when `configure` first ran (for the before/after comparison).
pub const FIRST_CONFIGURED_AT: &str = "first_configured_at";
/// Meta key: process start-up overhead measured by `doctor --bench`, in microseconds.
pub const BENCH_SPAWN_OVERHEAD_US: &str = "bench_spawn_overhead_us";

/// Everything read for one report.
pub struct Loaded {
    /// The database.
    pub store: Store,
    /// The Claude Code folder.
    pub claude_dir: PathBuf,
    /// Now.
    pub now: Timestamp,
    /// The requested period.
    pub range: TimeRange,
    /// The period in words (`last 7 days`).
    pub label: String,
    /// Every usage row (the projection and before/after look outside the period).
    pub usage: Vec<UsageRecord>,
    /// Every hook event.
    pub events: Vec<HookEventRecord>,
    /// The price table.
    pub prices: PriceTable,
    /// When `configure` first ran.
    pub first_configure: Option<Timestamp>,
    /// Process start-up overhead from the last `doctor --bench`.
    pub spawn_overhead_us: u64,
}

/// `7d` -> `last 7 days`, `12h` -> `last 12 hours`, `2w` -> `last 2 weeks`.
#[must_use]
pub fn span_label(span: &str) -> String {
    let span = span.trim();
    let split = span
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(span.len());
    let (number, unit) = span.split_at(split);
    let word = match unit {
        "h" => "hours",
        "d" => "days",
        "w" => "weeks",
        _ => return format!("last {span}"),
    };
    format!("last {number} {word}")
}

/// Ingests the transcripts and reads the data of the period `since`.
pub fn load(env: &Env, io: &mut Io, dir_flag: Option<PathBuf>, since: &str) -> Result<Loaded> {
    let claude_dir = select_claude_dir(&StdFileSystem, env, io, dir_flag, false)?;
    let store = existing_store(env)?
        .ok_or_else(|| anyhow!("no database yet; run `tokred configure` first"))?;
    let now = tokred_core::Clock::now(&SystemClock);
    let range =
        range_from_span(now, since).map_err(|err| anyhow!("invalid --since `{since}`: {err}"))?;

    let transcripts = StdTranscriptFs::new(claude_dir.join("projects"));
    ingest(&transcripts, &store, &store).context("could not read the Claude Code transcripts")?;

    let earliest = store.earliest_usage()?.unwrap_or(now);
    let end = Timestamp(now.0 + 1);
    let usage = store.usage_between(earliest, end)?;
    let events = store.events_between(Timestamp(0), end)?;
    let prices = load_table(&store, &SystemClock);
    let first_configure = store
        .meta_get(FIRST_CONFIGURED_AT)?
        .and_then(|value| value.parse::<i64>().ok())
        .map(Timestamp);
    let spawn_overhead_us = store
        .meta_get(BENCH_SPAWN_OVERHEAD_US)?
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    Ok(Loaded {
        store,
        claude_dir,
        now,
        range,
        label: span_label(since),
        usage,
        events,
        prices,
        first_configure,
        spawn_overhead_us,
    })
}

#[cfg(test)]
mod tests {
    use super::super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted;

    #[test]
    fn spans_are_described_in_words() {
        assert_eq!(span_label("7d"), "last 7 days");
        assert_eq!(span_label("12h"), "last 12 hours");
        assert_eq!(span_label(" 2w "), "last 2 weeks");
        assert_eq!(span_label("90m"), "last 90m");
        assert_eq!(span_label("x"), "last x");
    }

    #[test]
    fn it_ingests_the_transcripts_and_reads_everything() {
        let sb = Sandbox::new();
        sb.configure();
        sb.seed_usage();
        sb.seed_events();
        let (mut io, _) = scripted(false, "");

        let loaded = load(&sb.env, &mut io, Some(sb.claude.clone()), "7d").unwrap();
        assert_eq!(
            loaded.usage.len(),
            2,
            "the repeated streaming line is stored once"
        );
        assert_eq!(loaded.events.len(), 6);
        assert_eq!(loaded.label, "last 7 days");
        assert!(
            loaded.first_configure.is_some(),
            "configure records when it first ran"
        );
        assert_eq!(loaded.spawn_overhead_us, 0);
        assert!(
            !loaded.prices.entries().is_empty(),
            "the embedded snapshot is seeded"
        );

        // A second load is incremental: nothing is stored twice.
        let again = load(&sb.env, &mut io, Some(sb.claude.clone()), "7d").unwrap();
        assert_eq!(again.usage.len(), 2);
    }

    #[test]
    fn the_benchmarked_overhead_is_read_back() {
        let sb = Sandbox::new();
        sb.configure();
        sb.store()
            .meta_set(BENCH_SPAWN_OVERHEAD_US, "7000")
            .unwrap();
        let (mut io, _) = scripted(false, "");
        let loaded = load(&sb.env, &mut io, Some(sb.claude.clone()), "7d").unwrap();
        assert_eq!(loaded.spawn_overhead_us, 7_000);
    }

    #[test]
    fn problems_are_explained() {
        let sb = Sandbox::new();
        let (mut io, _) = scripted(false, "");
        let err = load(&sb.env, &mut io, Some(sb.claude.clone()), "7d")
            .err()
            .unwrap();
        assert!(err.to_string().contains("tokred configure"), "{err}");

        sb.configure();
        let err = load(&sb.env, &mut io, Some(sb.claude.clone()), "soon")
            .err()
            .unwrap();
        assert!(err.to_string().contains("invalid --since `soon`"), "{err}");

        let err = load(&sb.env, &mut io, Some(sb.path("nowhere")), "7d")
            .err()
            .unwrap();
        assert!(err.to_string().contains("not a Claude Code folder"));
    }
}
