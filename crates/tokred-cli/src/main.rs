//! The `tokred` binary.
//!
//! This is the single composition point of the workspace: it builds the real adapters
//! (file system, SQLite, clock, logging), calls the use cases and formats their output.

mod args;
mod binary;
mod commands;
mod env;
mod io;
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
    match commands::run(cli, &mut io::Io::stdio()) {
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
    std::panic::set_hook(Box::new(silent_panic_hook));
    let name = name.map(str::to_owned);
    let _ = std::panic::catch_unwind(move || {
        logging::init_for_hook();
        let output = name
            .as_deref()
            .and_then(HookEvent::from_arg)
            .and_then(|event| tokred_hooks::run_fail_open(event, &mut std::io::stdin().lock()));
        write_hook_output(output.as_deref(), &mut std::io::stdout().lock());
    });
    ExitCode::SUCCESS
}

/// Panic hook that prints nothing: a hook must never write to stderr either.
fn silent_panic_hook(_: &std::panic::PanicHookInfo<'_>) {}

/// Writes a hook's answer, if any. Write errors are ignored: a hook must never fail.
fn write_hook_output(output: Option<&str>, out: &mut dyn Write) {
    if let Some(text) = output {
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
}

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
    fn hook_output_is_written_verbatim() {
        let mut buffer = Vec::new();
        write_hook_output(Some("{\"ok\":true}"), &mut buffer);
        assert_eq!(buffer, b"{\"ok\":true}");
    }

    #[test]
    fn no_output_writes_nothing() {
        let mut buffer = Vec::new();
        write_hook_output(None, &mut buffer);
        assert!(buffer.is_empty());
    }

    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("closed pipe"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("closed pipe"))
        }
    }

    #[test]
    fn a_closed_stdout_is_ignored() {
        write_hook_output(Some("x"), &mut Broken);
    }
}
