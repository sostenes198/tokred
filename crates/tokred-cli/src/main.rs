//! The `tokred` binary.
//!
//! This is the single composition point of the workspace: it builds the real adapters
//! (file system, SQLite, clock, logging), calls the use cases and formats their output.

mod args;
mod binary;
mod commands;
mod env;
mod logging;

use std::io::Write;
use std::process::ExitCode;

use clap::Parser;
use tokred_core::HookEvent;

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().collect();
    // Hooks are parsed by hand and never fail: Claude Code treats exit code 2 as "block", so
    // a hook must exit 0 even when its arguments are unknown (e.g. after a downgrade).
    if raw.get(1).map(String::as_str) == Some("hook") {
        return run_hook(raw.get(2).map(String::as_str));
    }

    let cli = args::Cli::parse();
    match commands::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Runs `tokred hook <name>`. The *whole* body is inside `catch_unwind` (not just the handler),
/// so a panic while resolving paths or starting logging also ends in exit code 0.
fn run_hook(name: Option<&str>) -> ExitCode {
    std::panic::set_hook(Box::new(|_| {}));
    let name = name.map(str::to_owned);
    let _ = std::panic::catch_unwind(move || {
        logging::init_for_hook();
        if let Some(event) = name.as_deref().and_then(HookEvent::from_arg)
            && let Some(output) = tokred_hooks::run_fail_open(event, std::io::stdin().lock())
        {
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(output.as_bytes());
            let _ = stdout.flush();
        }
    });
    ExitCode::SUCCESS
}
