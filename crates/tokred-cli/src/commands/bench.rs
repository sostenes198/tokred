//! `tokred doctor --bench`: measures the real latency of every hook from the outside.
//!
//! Each hook is executed many times as a separate process with realistic input, in a scratch data
//! folder with its own database (so the hook does real work but your data is untouched), and timed
//! from spawn to exit. The difference between that and the time tokred measures inside itself is
//! the process start-up overhead; it is saved and used by `stats latency`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokred_core::Timestamp;
use tokred_core::hook_event::HookEventRepo;
use tokred_core::meta::MetaRepo;
use tokred_hooks::bench_fixtures;
use tokred_store::Store;

use super::data::BENCH_SPAWN_OVERHEAD_US;
use super::existing_store;
use crate::env::Env;

/// Hook runs per payload unless `TOKRED_BENCH_RUNS` says otherwise.
pub const DEFAULT_RUNS: usize = 50;
/// The latency budget of a hook, in milliseconds.
pub const BUDGET_MS: f64 = 50.0;

/// Runs one hook and returns how long it took, spawn to exit.
pub trait HookRunner {
    /// Runs `tokred hook <hook>` with `stdin` against the data folder `data_dir`.
    fn run(&self, hook: &str, stdin: &str, data_dir: &Path) -> Result<Duration>;
}

/// The real thing: spawns the tokred executable.
pub struct ProcessRunner<'a> {
    /// The executable to spawn.
    pub exe: &'a Path,
}

impl HookRunner for ProcessRunner<'_> {
    fn run(&self, hook: &str, stdin: &str, data_dir: &Path) -> Result<Duration> {
        let started = Instant::now();
        let mut child = std::process::Command::new(self.exe)
            .args(["hook", hook])
            .env("TOKRED_DATA_DIR", data_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("could not run {}", self.exe.display()))?;
        if let Some(mut pipe) = child.stdin.take() {
            // The hook may exit before reading everything; that is not an error here.
            let _ = pipe.write_all(stdin.as_bytes());
        }
        child.wait()?;
        Ok(started.elapsed())
    }
}

/// Timing of one hook.
#[derive(Debug, Clone, PartialEq)]
pub struct HookBench {
    /// Hook name.
    pub hook: String,
    /// Runs measured.
    pub runs: usize,
    /// Mean, spawn to exit, in ms.
    pub mean_ms: f64,
    /// Median in ms.
    pub p50_ms: f64,
    /// 95th percentile in ms.
    pub p95_ms: f64,
    /// Slowest run in ms.
    pub max_ms: f64,
}

/// Result of a benchmark.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchReport {
    /// Runs per payload.
    pub runs_per_payload: usize,
    /// One row per hook.
    pub hooks: Vec<HookBench>,
    /// Estimated process start-up overhead per run, in microseconds.
    pub spawn_overhead_us: u64,
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

#[allow(clippy::cast_precision_loss)]
fn to_ms(us: u64) -> f64 {
    us as f64 / 1_000.0
}

/// Nearest-rank percentile of sorted samples.
fn percentile(sorted: &[u64], p: usize) -> u64 {
    let rank = (p * sorted.len()).div_ceil(100).max(1);
    sorted.get(rank - 1).copied().unwrap_or(0)
}

/// Runs every payload `runs` times in `scratch` and summarizes per hook.
pub fn run_bench(runner: &dyn HookRunner, scratch: &Path, runs: usize) -> Result<BenchReport> {
    std::fs::create_dir_all(scratch)?;
    let store = Store::open(&scratch.join("tokred.db"))?;

    let mut samples: Vec<(String, Vec<u64>)> = Vec::new();
    for (event, payload) in bench_fixtures() {
        let name = event.arg();
        let index = if let Some(index) = samples.iter().position(|(hook, _)| hook == name) {
            index
        } else {
            samples.push((name.to_owned(), Vec::new()));
            samples.len() - 1
        };
        for _ in 0..runs {
            samples[index]
                .1
                .push(micros(runner.run(name, &payload, scratch)?));
        }
    }

    let hooks: Vec<HookBench> = samples
        .into_iter()
        .map(|(hook, mut us)| {
            us.sort_unstable();
            let mean = us.iter().sum::<u64>() / u64::try_from(us.len().max(1)).unwrap_or(1);
            HookBench {
                hook,
                runs: us.len(),
                mean_ms: to_ms(mean),
                p50_ms: to_ms(percentile(&us, 50)),
                p95_ms: to_ms(percentile(&us, 95)),
                max_ms: to_ms(us.last().copied().unwrap_or(0)),
            }
        })
        .collect();

    // What the hooks measured inside themselves, to isolate the start-up cost.
    let internal = store.events_between(Timestamp(0), Timestamp(i64::MAX / 2))?;
    let internal_mean = if internal.is_empty() {
        0
    } else {
        internal.iter().map(|e| e.duration_us).sum::<u64>()
            / u64::try_from(internal.len()).unwrap_or(1)
    };
    let external_total: f64 = hooks
        .iter()
        .map(|h| h.mean_ms * 1_000.0 * to_f64(h.runs))
        .sum();
    let external_runs: usize = hooks.iter().map(|h| h.runs).sum();
    let external_mean = if external_runs == 0 {
        0.0
    } else {
        external_total / to_f64(external_runs)
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let spawn_overhead_us = (external_mean - to_f64_u64(internal_mean)).max(0.0).round() as u64;

    Ok(BenchReport {
        runs_per_payload: runs,
        hooks,
        spawn_overhead_us,
    })
}

#[allow(clippy::cast_precision_loss)]
fn to_f64(value: usize) -> f64 {
    value as f64
}

#[allow(clippy::cast_precision_loss)]
fn to_f64_u64(value: u64) -> f64 {
    value as f64
}

/// Runs the benchmark in a scratch folder, saves the overhead and cleans up.
pub fn measure(env: &Env, runner: &dyn HookRunner, runs: usize) -> Result<BenchReport> {
    let scratch: PathBuf =
        std::env::temp_dir().join(format!("tokred-bench-{}", std::process::id()));
    let report = run_bench(runner, &scratch, runs);
    let _ = std::fs::remove_dir_all(&scratch);
    let report = report?;
    if let Some(store) = existing_store(env)? {
        let value = report.spawn_overhead_us.to_string();
        store.meta_set(BENCH_SPAWN_OVERHEAD_US, &value)?;
    }
    Ok(report)
}

/// `TOKRED_BENCH_RUNS`, else [`DEFAULT_RUNS`].
#[must_use]
pub fn runs_from_env() -> usize {
    std::env::var("TOKRED_BENCH_RUNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|runs| *runs > 0)
        .unwrap_or(DEFAULT_RUNS)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::super::testing::Sandbox;
    use super::*;

    /// Answers with scripted durations (in microseconds), cycling.
    struct Fake {
        script: Vec<u64>,
        calls: RefCell<Vec<String>>,
    }

    impl HookRunner for Fake {
        fn run(&self, hook: &str, _stdin: &str, _dir: &Path) -> Result<Duration> {
            let mut calls = self.calls.borrow_mut();
            calls.push(hook.to_owned());
            Ok(Duration::from_micros(
                self.script[calls.len() % self.script.len()],
            ))
        }
    }

    #[test]
    fn every_hook_is_measured_with_percentiles() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = Fake {
            script: vec![4_000, 10_000, 6_000, 8_000],
            calls: RefCell::default(),
        };
        let report = run_bench(&fake, &tmp.path().join("scratch"), 4).unwrap();

        let names: Vec<&str> = report.hooks.iter().map(|h| h.hook.as_str()).collect();
        for expected in [
            "route",
            "enforce",
            "guard-read",
            "guard-search",
            "session-check",
            "session-start",
        ] {
            assert!(
                names.contains(&expected),
                "{expected} missing from {names:?}"
            );
        }
        assert!(
            report
                .hooks
                .iter()
                .all(|h| h.runs >= 4 && h.max_ms >= h.p95_ms && h.p95_ms >= h.p50_ms)
        );
        assert!(
            report
                .hooks
                .iter()
                .all(|h| h.max_ms <= 10.0 && h.mean_ms >= 4.0)
        );
        assert_eq!(report.runs_per_payload, 4);
        assert!(
            report.spawn_overhead_us > 0,
            "no internal timings, so everything is overhead"
        );
        // Payloads of the same hook share a row.
        let unique: std::collections::BTreeSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
    }

    #[test]
    fn zero_runs_produce_an_empty_but_valid_report() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = Fake {
            script: vec![1],
            calls: RefCell::default(),
        };
        let report = run_bench(&fake, &tmp.path().join("s"), 0).unwrap();
        assert!(report.hooks.iter().all(|h| h.runs == 0 && h.max_ms == 0.0));
        assert_eq!(report.spawn_overhead_us, 0);
    }

    #[test]
    fn a_failing_runner_aborts_the_bench() {
        struct Broken;
        impl HookRunner for Broken {
            fn run(&self, _: &str, _: &str, _: &Path) -> Result<Duration> {
                anyhow::bail!("cannot spawn")
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        let err = run_bench(&Broken, &tmp.path().join("s"), 1).unwrap_err();
        assert!(err.to_string().contains("cannot spawn"));
    }

    #[test]
    fn measure_saves_the_overhead_and_cleans_up() {
        let sb = Sandbox::new();
        let (mut io, _) = crate::io::testing::scripted(false, "");
        super::super::configure::run(&sb.env, &mut io, Some(sb.claude.clone()), true, false)
            .unwrap();
        let fake = Fake {
            script: vec![5_000],
            calls: RefCell::default(),
        };
        let report = measure(&sb.env, &fake, 1).unwrap();

        let store = existing_store(&sb.env).unwrap().unwrap();
        assert_eq!(
            store.meta_get(BENCH_SPAWN_OVERHEAD_US).unwrap(),
            Some(report.spawn_overhead_us.to_string())
        );
        let scratch = std::env::temp_dir().join(format!("tokred-bench-{}", std::process::id()));
        assert!(!scratch.exists());

        // Without a database the numbers are still returned, just not saved.
        let bare = Sandbox::new();
        assert!(measure(&bare.env, &fake, 1).is_ok());
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        assert_eq!(percentile(&[5], 95), 5);
        assert_eq!(percentile(&[1, 2, 3, 4], 50), 2);
        assert_eq!(percentile(&[1, 2, 3, 4], 95), 4);
        assert_eq!(percentile(&[], 50), 0);
    }

    #[test]
    fn an_unrunnable_executable_is_reported() {
        let runner = ProcessRunner {
            exe: Path::new("/no/such/tokred"),
        };
        let err = runner.run("route", "{}", Path::new("/tmp")).unwrap_err();
        assert!(err.to_string().contains("could not run"));
    }

    #[test]
    fn the_run_count_can_be_set_from_the_environment_but_defaults_to_fifty() {
        assert_eq!(DEFAULT_RUNS, 50);
        assert!(runs_from_env() >= 1);
    }
}
