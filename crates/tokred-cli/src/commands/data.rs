//! Loading what `stats` and `metrics` need: ingest the transcripts, then read usage, proxy
//! requests, hook events, routing decisions, suggestions, prices and the first-configure time
//! from the store, and merge the two usage sources into one.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use tokred_core::Timestamp;
use tokred_core::hook_event::{HookEventRecord, HookEventRepo};
use tokred_core::meta::MetaRepo;
use tokred_core::pricing::PriceTable;
use tokred_core::prompts::{SuggestionRecord, SuggestionRepo};
use tokred_core::proxy::{ProxyRequestRecord, ProxyRequestRepo};
use tokred_core::routing::{RoutingDecision, RoutingRepo};
use tokred_core::usage::{UsageRecord, UsageRepo};
use tokred_install::StdFileSystem;
use tokred_metrics::proxy::{ProxyInput, ProxyStats, proxy_stats};
use tokred_metrics::{StdTranscriptFs, TimeRange, ingest, merge_usage, range_from_span};
use tokred_pricing::load_table;
use tokred_store::Store;

use super::{SystemClock, existing_store, select_claude_dir};
use crate::env::Env;
use crate::io::Io;

/// Meta key: when `configure` first ran (for the before/after comparison).
pub const FIRST_CONFIGURED_AT: &str = "first_configured_at";
/// Meta key: process start-up overhead measured by `doctor --bench`, in microseconds.
pub const BENCH_SPAWN_OVERHEAD_US: &str = "bench_spawn_overhead_us";

/// The repositories a report reads. Nothing else is needed, so the proxy process can build the
/// report from its own connection and a command can build it from the one it opened.
#[derive(Clone, Copy)]
pub struct Repos<'a> {
    /// Usage rows read from the transcripts.
    pub usage: &'a dyn UsageRepo,
    /// What the proxy measured, request by request.
    pub proxy: &'a dyn ProxyRequestRepo,
    /// Hook runs.
    pub events: &'a dyn HookEventRepo,
    /// Routing decisions.
    pub routing: &'a dyn RoutingRepo,
    /// Model suggestions.
    pub suggestions: &'a dyn SuggestionRepo,
    /// Small key/value state.
    pub meta: &'a dyn MetaRepo,
}

impl<'a> Repos<'a> {
    /// Every port backed by the one database.
    #[must_use]
    pub fn of(store: &'a Store) -> Self {
        Self {
            usage: store,
            proxy: store,
            events: store,
            routing: store,
            suggestions: store,
            meta: store,
        }
    }
}

/// Everything read for one report, apart from where it was read.
pub struct Gathered {
    /// Now.
    pub now: Timestamp,
    /// The requested period.
    pub range: TimeRange,
    /// The period in words (`last 7 days`).
    pub label: String,
    /// Every usage row, the transcripts and the proxy merged into one source (the projection and
    /// before/after look outside the period).
    pub usage: Vec<UsageRecord>,
    /// Every proxy request (the merge needs them all; the sections limit themselves to the period).
    pub requests: Vec<ProxyRequestRecord>,
    /// Routing decisions of the period.
    pub decisions: Vec<RoutingDecision>,
    /// Suggestions of the period.
    pub suggestions: Vec<SuggestionRecord>,
    /// Every hook event.
    pub events: Vec<HookEventRecord>,
    /// The price table.
    pub prices: PriceTable,
    /// When `configure` first ran.
    pub first_configure: Option<Timestamp>,
    /// Process start-up overhead from the last `doctor --bench`.
    pub spawn_overhead_us: u64,
}

impl Gathered {
    /// What only the proxy knows about the period; `None` when it recorded nothing.
    #[must_use]
    pub fn proxy(&self) -> Option<ProxyStats> {
        proxy_stats(&ProxyInput {
            usage: &self.usage,
            requests: &self.requests,
            decisions: &self.decisions,
            suggestions: &self.suggestions,
            prices: &self.prices,
            range: self.range,
        })
    }
}

/// Everything read for one command: the data and where it came from.
pub struct Loaded {
    /// The database.
    pub store: Store,
    /// The Claude Code folder.
    pub claude_dir: PathBuf,
    /// The data.
    pub data: Gathered,
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

/// Reads the data of the period `since` from the repositories and merges the usage sources. It
/// reads only: the transcripts are ingested by [`load`], and the proxy writes its own rows.
pub fn gather(
    repos: &Repos<'_>,
    prices: PriceTable,
    now: Timestamp,
    since: &str,
) -> Result<Gathered> {
    let range =
        range_from_span(now, since).map_err(|err| anyhow!("invalid --since `{since}`: {err}"))?;
    let earliest = repos.usage.earliest_usage()?.unwrap_or(now);
    let end = Timestamp(now.0 + 1);
    let transcripts = repos.usage.usage_between(earliest, end)?;
    let requests = repos.proxy.requests_between(Timestamp(0), end)?;
    let usage = merge_usage(&transcripts, &requests).records;
    let first_configure = repos
        .meta
        .meta_get(FIRST_CONFIGURED_AT)?
        .and_then(|value| value.parse::<i64>().ok())
        .map(Timestamp);
    let spawn_overhead_us = repos
        .meta
        .meta_get(BENCH_SPAWN_OVERHEAD_US)?
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    Ok(Gathered {
        now,
        range,
        label: span_label(since),
        usage,
        requests,
        decisions: repos.routing.decisions_between(range.from, range.to)?,
        suggestions: repos
            .suggestions
            .suggestions_between(range.from, range.to)?,
        events: repos.events.events_between(Timestamp(0), end)?,
        prices,
        first_configure,
        spawn_overhead_us,
    })
}

/// Ingests the transcripts and reads the data of the period `since`.
pub fn load(env: &Env, io: &mut Io, dir_flag: Option<PathBuf>, since: &str) -> Result<Loaded> {
    let claude_dir = select_claude_dir(&StdFileSystem, env, io, dir_flag, false)?;
    load_from(env, claude_dir, since)
}

/// [`load`] for a Claude Code folder that is already known.
pub fn load_from(env: &Env, claude_dir: PathBuf, since: &str) -> Result<Loaded> {
    let store = existing_store(env)?
        .ok_or_else(|| anyhow!("no database yet; run `tokred configure` first"))?;
    let now = tokred_core::Clock::now(&SystemClock);
    // A bad `--since` is reported before the transcripts are read.
    range_from_span(now, since).map_err(|err| anyhow!("invalid --since `{since}`: {err}"))?;

    let transcripts = StdTranscriptFs::new(claude_dir.join("projects"));
    ingest(&transcripts, &store, &store).context("could not read the Claude Code transcripts")?;

    let prices = load_table(&store, &SystemClock);
    let data = gather(&Repos::of(&store), prices, now, since)?;
    Ok(Loaded {
        store,
        claude_dir,
        data,
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
            loaded.data.usage.len(),
            2,
            "the repeated streaming line is stored once"
        );
        assert_eq!(loaded.data.events.len(), 6);
        assert_eq!(loaded.data.label, "last 7 days");
        assert!(
            loaded.data.first_configure.is_some(),
            "configure records when it first ran"
        );
        assert_eq!(loaded.data.spawn_overhead_us, 0);
        assert!(
            !loaded.data.prices.entries().is_empty(),
            "the embedded snapshot is seeded"
        );

        // A second load is incremental: nothing is stored twice.
        let again = load(&sb.env, &mut io, Some(sb.claude.clone()), "7d").unwrap();
        assert_eq!(again.data.usage.len(), 2);
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
        assert_eq!(loaded.data.spawn_overhead_us, 7_000);
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
    #[test]
    fn without_the_proxy_the_usage_is_what_the_transcripts_say() {
        let sb = Sandbox::new();
        sb.configure();
        sb.seed_usage();
        let (mut io, _) = scripted(false, "");
        let loaded = load(&sb.env, &mut io, Some(sb.claude.clone()), "7d").unwrap();
        let data = &loaded.data;
        assert!(data.requests.is_empty() && data.decisions.is_empty());
        assert!(data.usage.iter().all(|row| row.origin.is_none()));
        assert!(data.proxy().is_none(), "no proxy rows, no proxy sections");
    }

    #[test]
    fn the_proxy_and_the_transcripts_are_one_source() {
        let sb = Sandbox::new();
        sb.configure();
        sb.seed_usage();
        sb.seed_proxy();
        let (mut io, _) = scripted(false, "");
        let loaded = load(&sb.env, &mut io, Some(sb.claude.clone()), "7d").unwrap();
        let data = &loaded.data;

        // Two transcript messages, three proxy requests, two of them the same messages.
        assert_eq!(data.requests.len(), 3);
        assert_eq!(data.usage.len(), 3, "a proxied message is counted once");
        let row = |key: &str| {
            data.usage
                .iter()
                .find(|row| row.message_key == key)
                .unwrap()
        };
        assert_eq!(row("m2:r-m2").model, "claude-haiku-4-5", "what answered");
        assert_eq!(
            row("m2:r-m2").project,
            "-tmp-proj",
            "the transcript's project"
        );
        assert_eq!(
            row("m1:r-m1").usage.cache_write_1h,
            40,
            "the proxy's 1-hour split"
        );
        assert_eq!(row("m9:r-m9").origin.as_deref(), Some("auxiliary"));
        assert_eq!(data.decisions.len(), 2);
        assert_eq!(data.suggestions.len(), 1);

        let proxy = data.proxy().unwrap();
        assert_eq!(proxy.coverage.measured, 3);
        assert_eq!(proxy.coverage.transcript_only, 0);
    }

    #[test]
    fn reading_validates_the_period_too() {
        let sb = Sandbox::new();
        sb.configure();
        let store = sb.store();
        let err = gather(
            &Repos::of(&store),
            PriceTable::default(),
            Timestamp(1_000),
            "soon",
        )
        .err()
        .unwrap();
        assert!(err.to_string().contains("invalid --since `soon`"), "{err}");
    }
}
