//! The `tokred` binary.
//!
//! This is the single composition point of the workspace: it builds the real adapters
//! (file system, SQLite, clock, logging), calls the use cases and formats their output.

mod args;
mod binary;
mod commands;
mod daemon;
mod env;
mod io;
mod logging;
mod term;
mod view;

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::Parser;
use tokred_core::hook_event::HookEventRepo;
use tokred_core::{Config, HookEvent};
use tokred_hooks::fs::{StdFileMeta, SystemClock};
use tokred_hooks::{HookDeps, HookRun};
use tokred_store::Store;

/// How long a hook may run before the time guard gives up (Claude Code's own timeout is 5 s).
const DEFAULT_GUARD_MS: u64 = 2000;

fn main() -> ExitCode {
    // Hook latency is measured from here: the first line of `main`.
    let start = Instant::now();
    let raw: Vec<String> = std::env::args().collect();
    // Hooks are parsed by hand and never fail: Claude Code treats exit code 2 as "block", so
    // a hook must exit 0 even when its arguments are unknown (e.g. after a downgrade).
    if raw.get(1).map(String::as_str) == Some("hook") {
        return run_hook(raw.get(2).map(String::as_str), start);
    }

    let cli = args::Cli::parse();
    match commands::run(cli, &mut io::Io::stdio()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => exit_code_for(&err),
    }
}

/// The exit code for a failed command: the child's own code for `tokred claude`, else 1 after
/// printing the error.
fn exit_code_for(err: &anyhow::Error) -> ExitCode {
    if let Some(child) = err.downcast_ref::<commands::ChildExit>() {
        return ExitCode::from(u8::try_from(child.0).unwrap_or(1));
    }
    eprintln!("error: {err:#}");
    ExitCode::FAILURE
}

/// Runs `tokred hook <name>`. The *whole* body is inside `catch_unwind` (not just the handler),
/// so a panic while resolving paths or starting logging also ends in exit code 0.
fn run_hook(name: Option<&str>, start: Instant) -> ExitCode {
    std::panic::set_hook(Box::new(silent_panic_hook));
    let name = name.map(str::to_owned);
    let _ = std::panic::catch_unwind(move || {
        logging::init_for_hook();
        compose_and_run_hook(name.as_deref(), start);
    });
    ExitCode::SUCCESS
}

/// Builds the real adapters and runs the hook. Does nothing when the data folder or the
/// database is missing: a hook never creates them (they are made by `tokred configure`, and a
/// purged installation must stay purged).
fn compose_and_run_hook(name: Option<&str>, start: Instant) {
    let (Some(event), Ok(env)) = (name.and_then(HookEvent::from_arg), env::Env::from_process())
    else {
        return;
    };
    let db = env.data.db_path();
    if !db.is_file() {
        return;
    }
    // Time guard first, so that even a blocked stdin or a locked database cannot hold Claude
    // Code up: it records `timeout_guard` and exits 0 without output.
    let guard_db = db.clone();
    let _guard = tokred_hooks::watchdog::arm(guard_limit(), move || {
        if let Ok(store) = Store::open(&guard_db) {
            tokred_hooks::record_timeout(&store, &SystemClock, event, start);
        }
        std::process::exit(0);
    });

    let config = read_config(&env.data.config_path());
    let store = Store::open(&db).ok();
    let files = StdFileMeta::default();
    let deps = store.as_ref().map(|store| HookDeps {
        clock: &SystemClock,
        files: &files,
        sessions: store,
        reads: store,
        classes: store,
        cache: store,
        config: &config,
    });
    let run = HookRun {
        deps: deps.as_ref(),
        events: store.as_ref().map(|store| store as &dyn HookEventRepo),
        clock: &SystemClock,
        start,
    };
    tokred_hooks::run_hook(
        name,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        &run,
    );
}

/// The time guard limit: `TOKRED_HOOK_GUARD_MS` (used by the tests), else 2 s.
fn guard_limit() -> Duration {
    let ms = std::env::var("TOKRED_HOOK_GUARD_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_GUARD_MS);
    Duration::from_millis(ms)
}

/// `tokred.toml`, or the defaults when it is missing or invalid (fail-open).
fn read_config(path: &Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(text) => Config::from_toml_str(&text).unwrap_or_else(|err| {
            tracing::error!(error = %err, "invalid tokred.toml, using defaults");
            Config::default()
        }),
        Err(_) => Config::default(),
    }
}

/// Panic hook that prints nothing: a hook must never write to stderr either.
fn silent_panic_hook(_: &std::panic::PanicHookInfo<'_>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_silent_panic_hook_is_silent() {
        let result = std::panic::catch_unwind(|| {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(silent_panic_hook));
            let _ = std::panic::catch_unwind(|| panic!("this must not be printed"));
            std::panic::set_hook(previous);
        });
        assert!(result.is_ok());
    }

    #[test]
    fn a_missing_or_invalid_config_means_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tokred.toml");
        assert_eq!(read_config(&path), Config::default());
        std::fs::write(&path, "[modes\nbroken").unwrap();
        assert_eq!(read_config(&path), Config::default());
        std::fs::write(&path, "[modes]\nguard_read = \"off\"\n").unwrap();
        assert_eq!(
            read_config(&path).modes.guard_read,
            tokred_core::config::Mode::Off
        );
    }

    #[test]
    fn the_guard_limit_has_a_default_and_an_override() {
        // Setting an environment variable needs `unsafe` on edition 2024, which is forbidden
        // here: the override is covered end to end in the e2e suite and only the default is
        // checked in-process.
        assert!(
            std::env::var_os("TOKRED_HOOK_GUARD_MS").is_some()
                || guard_limit() == Duration::from_millis(DEFAULT_GUARD_MS)
        );
    }
}
