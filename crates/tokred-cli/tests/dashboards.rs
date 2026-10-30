//! End-to-end tests of the terminal dashboards, the HTML report, the cache and `doctor --bench`,
//! running the real binary in a sandbox with a seeded transcript.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("my claude/projects/-tmp-proj")).unwrap();
        std::fs::create_dir_all(root.path().join("home")).unwrap();
        std::fs::write(root.path().join("my claude/settings.json"), "{}").unwrap();
        // A fixed past timestamp; the tests ask for a very long period.
        let line = |id: &str, model: &str, input: u64| {
            format!(
                r#"{{"type":"assistant","sessionId":"s1","timestamp":"2026-01-15T12:00:00.000Z","requestId":"r-{id}","cwd":"/p","isSidechain":false,"message":{{"id":"{id}","model":"{model}","usage":{{"input_tokens":{input},"output_tokens":{},"cache_creation_input_tokens":100,"cache_read_input_tokens":2000}}}}}}"#,
                input / 2
            )
        };
        let text = [
            line("m1", "claude-opus-4-1-20250805", 200_000),
            line("m2", "claude-sonnet-4-20250514", 300_000),
        ]
        .join("\n");
        std::fs::write(
            root.path().join("my claude/projects/-tmp-proj/s1.jsonl"),
            format!("{text}\n"),
        )
        .unwrap();
        Self { root }
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("tokred data")
    }

    fn tokred(&self) -> Command {
        let home = self.root.path().join("home");
        let mut cmd = cargo_bin_cmd!("tokred");
        cmd.env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", home.join("appdata"))
            .env("XDG_DATA_HOME", home.join("xdg"))
            .env("CLAUDE_CONFIG_DIR", self.root.path().join("my claude"))
            .env("TOKRED_DATA_DIR", self.data())
            .env("TOKRED_OFFLINE", "1")
            .env("TOKRED_BENCH_RUNS", "2")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("TOKRED_LOG");
        cmd
    }

    fn configure(&self) {
        self.tokred()
            .args(["configure", "--yes"])
            .assert()
            .success();
    }
}

#[test]
fn stats_prints_the_dashboard_without_color_when_piped() {
    let sb = Sandbox::new();
    sb.configure();
    sb.tokred()
        .args(["stats", "--since", "3650d"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "tokred Token Savings (last 3650 days)",
        ))
        .stdout(predicate::str::contains("Efficiency meter:"))
        .stdout(predicate::str::contains("claude-sonnet-4-20250514"))
        .stdout(predicate::str::contains("\u{1b}[").not());
}

#[test]
fn colors_follow_clicolor_force_and_no_color() {
    let sb = Sandbox::new();
    sb.configure();
    sb.tokred()
        .env("CLICOLOR_FORCE", "1")
        .args(["stats", "--since", "3650d"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\u{1b}[1;36m"));
    sb.tokred()
        .env("CLICOLOR_FORCE", "1")
        .env("NO_COLOR", "1")
        .args(["stats", "--since", "3650d"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\u{1b}[").not());
}

#[test]
fn stats_json_and_grouping() {
    let sb = Sandbox::new();
    sb.configure();
    let out = sb
        .tokred()
        .args(["stats", "--since", "3650d", "--by", "day", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["stats"]["totals"]["messages"], 2);
    assert_eq!(json["stats"]["group_by"], "day");
    sb.tokred()
        .args(["stats", "--since", "banana"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid --since"));
}

#[test]
fn stats_needs_a_configured_install() {
    let sb = Sandbox::new();
    sb.tokred()
        .arg("stats")
        .assert()
        .failure()
        .stderr(predicate::str::contains("tokred configure"));
}

#[test]
fn metrics_writes_the_html_report_and_prints_its_path() {
    let sb = Sandbox::new();
    sb.configure();
    sb.tokred()
        .args(["metrics", "--since", "3650d", "--no-open"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tokred Token Savings"))
        .stdout(predicate::str::contains("HTML report written to"));
    let html = std::fs::read_to_string(sb.data().join("reports/metrics.html")).unwrap();
    assert!(html.contains("claude-opus-4-1-20250805"));
    assert!(
        !html.contains("http://") && !html.contains("https://"),
        "offline report"
    );
}

#[test]
fn stats_latency_without_runs_explains_what_to_do() {
    let sb = Sandbox::new();
    sb.configure();
    sb.tokred()
        .args(["stats", "latency"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tokred Hook Latency"))
        .stdout(predicate::str::contains("No hook runs recorded"));
}

#[test]
fn cache_commands_work_on_an_empty_cache() {
    let sb = Sandbox::new();
    sb.configure();
    sb.tokred()
        .args(["cache", "stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("The cache is empty."));
    sb.tokred()
        .args(["cache", "prune"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed 0 expired entries."));
}

#[test]
fn doctor_bench_measures_every_hook_and_saves_the_overhead() {
    let sb = Sandbox::new();
    sb.configure();
    sb.tokred()
        .args(["doctor", "--bench"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "tokred Hook Benchmark (2 runs per payload)",
        ))
        .stdout(predicate::str::contains("guard-read"))
        .stdout(predicate::str::contains("Process start-up overhead"));

    let out = sb
        .tokred()
        .args(["doctor", "--bench", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["bench"]["hooks"].as_array().unwrap().len(), 7);
    assert_eq!(json["bench"]["budget_ms"], 50.0);
    assert!(json["bench"]["spawn_overhead_us"].as_u64().is_some());

    // The bench ran in a scratch folder: the real database has no benchmark sessions.
    sb.tokred()
        .args(["stats", "latency", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"runs\": 0"));
}
