//! Hook handlers invoked by Claude Code as `tokred hook <event>`.
//!
//! Every handler is **fail-open**: on error, panic or bad input the hook produces no output
//! and the caller exits with code 0, so a tokred bug can never block Claude Code.
//!
//! In phase F0 the handlers are stubs: they drain stdin and decide nothing. The real
//! behaviour arrives in phases F3 to F5 (see `docs/spec.md`).

use std::io::Read;
use std::panic::{AssertUnwindSafe, catch_unwind};

use tokred_core::HookEvent;

/// Upper bound on the stdin payload a hook reads (transcripts are never sent on stdin).
const MAX_STDIN_BYTES: u64 = 1024 * 1024;

/// Runs a hook and returns the text to print on stdout, if any.
///
/// Never panics and never fails: any problem is logged and turned into "no output".
pub fn run_fail_open(event: HookEvent, stdin: &mut dyn Read) -> Option<String> {
    match catch_unwind(AssertUnwindSafe(|| handle(event, stdin))) {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            tracing::error!(hook = event.arg(), error = %err, "hook failed (fail-open)");
            None
        }
        Err(_) => {
            tracing::error!(hook = event.arg(), "hook panicked (fail-open)");
            None
        }
    }
}

fn handle(event: HookEvent, stdin: &mut dyn Read) -> std::io::Result<Option<String>> {
    let mut payload = Vec::new();
    Read::take(&mut *stdin, MAX_STDIN_BYTES).read_to_end(&mut payload)?;
    tracing::debug!(
        hook = event.arg(),
        bytes = payload.len(),
        "hook stub: no decision"
    );
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_emits_nothing_for_every_event() {
        for event in HookEvent::ALL {
            assert_eq!(
                run_fail_open(event, &mut &b"{\"session_id\":\"s\"}"[..]),
                None
            );
        }
    }

    #[test]
    fn the_debug_trace_is_emitted_without_changing_the_result() {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(std::io::sink)
            .finish();
        let out = tracing::subscriber::with_default(subscriber, || {
            run_fail_open(HookEvent::GuardRead, &mut &b"{}"[..])
        });
        assert_eq!(out, None);
    }

    struct Failing;
    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("boom"))
        }
    }

    /// Runs `f` with a subscriber that accepts every level, so log fields are evaluated.
    fn logged<T>(f: impl FnOnce() -> T) -> T {
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(std::io::sink)
            .finish();
        tracing::subscriber::with_default(subscriber, f)
    }

    #[test]
    fn read_errors_are_swallowed() {
        assert_eq!(
            logged(|| run_fail_open(HookEvent::GuardRead, &mut Failing)),
            None
        );
    }

    struct Panicking;
    impl Read for Panicking {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("boom");
        }
    }

    #[test]
    fn panics_are_swallowed() {
        assert_eq!(
            logged(|| run_fail_open(HookEvent::Route, &mut Panicking)),
            None
        );
    }
}
