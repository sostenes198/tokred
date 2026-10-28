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
pub fn run_fail_open<R: Read>(event: HookEvent, stdin: R) -> Option<String> {
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

fn handle<R: Read>(event: HookEvent, stdin: R) -> std::io::Result<Option<String>> {
    let mut payload = Vec::new();
    stdin.take(MAX_STDIN_BYTES).read_to_end(&mut payload)?;
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
            assert_eq!(run_fail_open(event, &b"{\"session_id\":\"s\"}"[..]), None);
        }
    }

    struct Failing;
    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("boom"))
        }
    }

    #[test]
    fn read_errors_are_swallowed() {
        assert_eq!(run_fail_open(HookEvent::GuardRead, Failing), None);
    }

    struct Panicking;
    impl Read for Panicking {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("boom");
        }
    }

    #[test]
    fn panics_are_swallowed() {
        assert_eq!(run_fail_open(HookEvent::Route, Panicking), None);
    }
}
