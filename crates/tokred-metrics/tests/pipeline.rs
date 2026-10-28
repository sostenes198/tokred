//! Ingestion from real files into fake repositories, then `stats` against a manual sum.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use tokred_core::hook_event::{HookEventRecord, Outcome};
use tokred_core::pricing::{ModelPrice, PriceEntry, PriceTable};
use tokred_core::usage::{IngestRepo, IngestState, UsageRecord, UsageRepo};
use tokred_core::{StoreError, Timestamp};
use tokred_metrics::{
    GroupBy, StatsInput, StdTranscriptFs, TimeRange, compute_stats, guards_summary, ingest,
    latency_stats, routing_summary, tradeoff,
};

/// 2026-03-01T00:00:00Z.
const T0: i64 = 1_772_323_200;
const OPUS: &str = "claude-opus-4-1-20250805";
const SONNET: &str = "claude-sonnet-4-20250514";

#[derive(Default)]
struct MemoryRepos {
    rows: RefCell<BTreeMap<String, UsageRecord>>,
    states: RefCell<BTreeMap<String, IngestState>>,
}

impl UsageRepo for MemoryRepos {
    fn insert_new(&self, records: &[UsageRecord]) -> Result<usize, StoreError> {
        let mut rows = self.rows.borrow_mut();
        let before = rows.len();
        for record in records {
            rows.entry(record.message_key.clone())
                .or_insert_with(|| record.clone());
        }
        Ok(rows.len() - before)
    }

    fn usage_between(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<UsageRecord>, StoreError> {
        let mut rows: Vec<_> = self
            .rows
            .borrow()
            .values()
            .filter(|r| from <= r.ts && r.ts < to)
            .cloned()
            .collect();
        rows.sort_by_key(|r| r.ts);
        Ok(rows)
    }

    fn earliest_usage(&self) -> Result<Option<Timestamp>, StoreError> {
        Ok(self.rows.borrow().values().map(|r| r.ts).min())
    }
}

impl IngestRepo for MemoryRepos {
    fn ingest_state(&self, file: &str) -> Result<Option<IngestState>, StoreError> {
        Ok(self.states.borrow().get(file).cloned())
    }

    fn save_ingest_state(&self, state: &IngestState) -> Result<(), StoreError> {
        self.states
            .borrow_mut()
            .insert(state.file.clone(), state.clone());
        Ok(())
    }
}

fn prices() -> PriceTable {
    let entry = |model: &str, input, output, cache_write, cache_read| PriceEntry {
        model: model.to_owned(),
        price: ModelPrice {
            input,
            output,
            cache_write,
            cache_write_1h: input * 2.0,
            cache_read,
        },
        source: "fixture".to_owned(),
        updated_at: Timestamp(0),
    };
    PriceTable::new(vec![
        entry(OPUS, 15.0, 75.0, 18.75, 1.5),
        entry(SONNET, 3.0, 15.0, 3.75, 0.3),
    ])
}

fn assistant(
    id: &str,
    request: &str,
    session: &str,
    second: i64,
    model: &str,
    tokens: [u64; 4],
    sidechain: bool,
) -> String {
    let (hour, minute, sec) = (second / 3600, second % 3600 / 60, second % 60);
    format!(
        r#"{{"parentUuid":null,"isSidechain":{sidechain},"userType":"external","cwd":"/Users/me/app","sessionId":"{session}","version":"2.1.0","type":"assistant","timestamp":"2026-03-01T{hour:02}:{minute:02}:{sec:02}.250Z","requestId":"{request}","uuid":"u-{id}","message":{{"id":"{id}","type":"message","role":"assistant","model":"{model}","content":[{{"type":"text","text":"hi"}}],"usage":{{"input_tokens":{},"output_tokens":{},"cache_creation_input_tokens":{},"cache_read_input_tokens":{},"service_tier":"standard"}}}}}}"#,
        tokens[0], tokens[1], tokens[2], tokens[3]
    )
}

fn write_lines(path: &Path, lines: &[String]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, lines.join("\n") + "\n").unwrap();
}

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    /// Main session (opus, sonnet, a streamed duplicate, noise lines) plus one subagent file.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("-Users-me-app");
        write_lines(
            &project.join("sess1.jsonl"),
            &[
                r#"{"type":"queue-operation","operation":"enqueue","timestamp":"2026-03-01T00:00:00Z"}"#.to_owned(),
                r#"{"type":"user","sessionId":"sess1","timestamp":"2026-03-01T00:00:01Z","message":{"role":"user","content":"hello"}}"#.to_owned(),
                assistant("m1", "r1", "sess1", 10, OPUS, [1_000, 500, 2_000, 10_000], false),
                assistant("m1", "r1", "sess1", 11, OPUS, [1_000, 500, 2_000, 10_000], false),
                "{ this is not json".to_owned(),
                assistant("m2", "r2", "sess1", 20, SONNET, [2_000, 1_000, 0, 8_000], false),
                assistant("m3", "r3", "sess1", 30, "<synthetic>", [0, 0, 0, 0], false),
                r#"{"type":"attachment","timestamp":"2026-03-01T00:00:31Z"}"#.to_owned(),
            ],
        );
        write_lines(
            &project.join("sess1/subagents/agent-a1.jsonl"),
            &[assistant(
                "s1",
                "q1",
                "sess1",
                40,
                SONNET,
                [1_000_000, 100_000, 0, 0],
                true,
            )],
        );
        Self { dir }
    }

    fn fs(&self) -> StdTranscriptFs {
        StdTranscriptFs::new(self.dir.path())
    }
}

fn events() -> Vec<HookEventRecord> {
    let event =
        |hook: &str, decision: &str, reason: &str, bytes: u64, us: u64, at: i64| HookEventRecord {
            ts: Timestamp(T0 + at),
            session_id: "sess1".to_owned(),
            hook: hook.to_owned(),
            decision: decision.to_owned(),
            reason: reason.to_owned(),
            bytes_saved: bytes,
            duration_us: us,
            outcome: Outcome::Ok,
        };
    vec![
        event("route", "implementation", "suggested", 0, 1_200, 5),
        event("route", "architecture", "suggested", 0, 900, 6),
        event("enforce", "allow", "agent_matches", 0, 700, 7),
        event("enforce", "deny", "agent_mismatch", 0, 800, 8),
        event("guard-read", "deny", "reread", 40_000, 2_000, 9),
        event("guard-read", "deny", "large_file", 80_000, 3_000, 9),
        event("guard-search", "warn", "broad_search", 0, 1_500, 9),
    ]
}

fn range() -> TimeRange {
    TimeRange::new(Timestamp(T0), Timestamp(T0 + 86_400))
}

#[test]
fn stats_match_a_manual_sum_over_the_ingested_transcripts() {
    let fixture = Fixture::new();
    let repos = MemoryRepos::default();
    let summary = ingest(&fixture.fs(), &repos, &repos).unwrap();

    // Streamed duplicate, malformed, synthetic and non-assistant lines are dropped.
    assert_eq!(summary.files_seen, 2);
    assert_eq!(summary.files_read, 2);
    assert_eq!(
        summary.records_found, 4,
        "m1 twice, m2 and the subagent line"
    );
    assert_eq!(summary.records_inserted, 3);

    let rows = repos
        .usage_between(Timestamp(0), Timestamp(i64::MAX))
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter()
            .all(|r| r.project == "-Users-me-app" && r.session_id == "sess1")
    );
    assert_eq!(rows.iter().filter(|r| r.is_subagent).count(), 1);
    assert_eq!(repos.earliest_usage().unwrap(), Some(Timestamp(T0 + 10)));

    let usage = repos
        .usage_between(Timestamp(0), Timestamp(i64::MAX))
        .unwrap();
    let stats = compute_stats(&StatsInput {
        usage: &usage,
        prices: &prices(),
        events: &events(),
        range: range(),
        group_by: GroupBy::Model,
        now: Timestamp(T0 + 3_600),
        first_configure: Some(Timestamp(T0 + 15)),
    });
    // By hand: opus 0.105 + sonnet 0.0234 + subagent sonnet (3 + 1.5) = 4.6284.
    assert_eq!(stats.totals.cost_usd, 4.6284);
    assert_eq!(stats.totals.tokens.total, 13_500 + 11_000 + 1_100_000);
    assert_eq!((stats.totals.messages, stats.totals.sessions), (3, 1));
    // Same tokens on opus: 0.105 + 0.117 + 22.5.
    assert_eq!(stats.savings.opus_cost_usd, Some(22.722));
    assert_eq!(stats.savings.routing_saving_usd, Some(18.0936));
    // Guards: 120_000 bytes -> 30_000 tokens at the session's model (sonnet, 3 messages: 2 sonnet
    // 1 opus) input price: 30_000 x 3 / 1e6.
    assert_eq!(stats.savings.guard.tokens_avoided, 30_000);
    assert_eq!(stats.savings.guard.saving_usd, 0.09);
    assert!(stats.unpriced_models.is_empty());

    insta::assert_json_snapshot!("stats", stats);
}

#[test]
fn ingestion_resumes_and_waits_for_partial_lines() {
    let fixture = Fixture::new();
    let repos = MemoryRepos::default();
    ingest(&fixture.fs(), &repos, &repos).unwrap();

    let again = ingest(&fixture.fs(), &repos, &repos).unwrap();
    assert_eq!((again.files_read, again.records_inserted), (0, 0));

    // Claude Code is mid-write: half a line with no newline yet.
    let path = fixture.dir.path().join("-Users-me-app/sess1.jsonl");
    let full = assistant("m4", "r4", "sess1", 50, OPUS, [7, 0, 0, 0], false);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(&full.as_bytes()[..60]).unwrap();
    let partial = ingest(&fixture.fs(), &repos, &repos).unwrap();
    assert_eq!(partial.records_inserted, 0);

    // The rest arrives.
    file.write_all(&full.as_bytes()[60..]).unwrap();
    file.write_all(b"\n").unwrap();
    let done = ingest(&fixture.fs(), &repos, &repos).unwrap();
    assert_eq!((done.records_found, done.records_inserted), (1, 1));
    assert_eq!(repos.rows.borrow().len(), 4);

    // The file is rotated to something shorter: re-read from the start, nothing duplicated.
    write_lines(
        &path,
        &[assistant(
            "m9",
            "r9",
            "sess1",
            60,
            OPUS,
            [1, 1, 1, 1],
            false,
        )],
    );
    let rotated = ingest(&fixture.fs(), &repos, &repos).unwrap();
    assert_eq!(rotated.records_inserted, 1);
    assert_eq!(repos.rows.borrow().len(), 5);
}

#[test]
fn a_missing_projects_directory_ingests_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let repos = MemoryRepos::default();
    let fs = StdTranscriptFs::new(dir.path().join("nope"));
    let summary = ingest(&fs, &repos, &repos).unwrap();
    assert_eq!(summary.files_seen, 0);
}

#[test]
fn latency_routing_and_guards_snapshots() {
    let latency = latency_stats(&events(), 2_000, range());
    insta::assert_json_snapshot!("latency", latency);

    insta::assert_json_snapshot!("routing", routing_summary(&events(), range()));
    insta::assert_json_snapshot!("guards", guards_summary(&events(), range()));

    let stats = compute_stats(&StatsInput {
        usage: &[],
        prices: &prices(),
        events: &events(),
        range: range(),
        group_by: GroupBy::Day,
        now: Timestamp(T0),
        first_configure: None,
    });
    insta::assert_json_snapshot!("tradeoff", tradeoff(&latency, &stats));
}
