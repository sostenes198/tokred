//! The hook runtime: reads stdin, dispatches to the handler, writes the answer and, as the very
//! last step, records the execution in `hook_events`.
//!
//! Everything is fail-open. A parse error, a store error, a panic or a closed stdout ends in
//! "no output" (the caller exits 0). The row in `hook_events` is written *after* the answer was
//! written and flushed, so the write is not part of the delay Claude Code perceives. (The
//! process cannot close its own stdout: `unsafe` is forbidden in this workspace, so a file
//! descriptor cannot be closed early. The answer is flushed, and Claude Code reads it as soon
//! as the process exits; the write is a fraction of a millisecond.)

use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;

use tokred_core::Clock;
use tokred_core::HookEvent;
use tokred_core::hook_event::{HookEventRecord, HookEventRepo, Outcome};

use crate::error::HookError;
use crate::guard::{guard_read, guard_search};
use crate::io::HookInput;
use crate::ports::{EventNote, HookDeps, HookResponse};
use crate::route::{enforce, route};
use crate::session::{session_check, session_start};

/// Upper bound on the stdin payload a hook reads (transcripts are never sent on stdin).
pub const MAX_STDIN_BYTES: u64 = 1024 * 1024;

/// Parses the payload and runs the handler of `event`.
pub fn handle(
    event: HookEvent,
    payload: &[u8],
    deps: &HookDeps<'_>,
) -> Result<HookResponse, HookError> {
    let input = HookInput::parse(payload)?;
    match event {
        HookEvent::Route => route(&input, deps),
        HookEvent::Enforce => enforce(&input, deps),
        HookEvent::GuardRead => guard_read(&input, deps),
        HookEvent::GuardSearch => guard_search(&input, deps),
        HookEvent::SessionCheck => session_check(&input, deps),
        HookEvent::SessionStart => session_start(&input, deps),
    }
}

/// What [`run_hook`] works with. `deps` and `events` are `None` when the store could not be
/// opened: the hook then does nothing (fail-open).
pub struct HookRun<'a> {
    /// Handler dependencies.
    pub deps: Option<&'a HookDeps<'a>>,
    /// Where to record the execution.
    pub events: Option<&'a dyn HookEventRepo>,
    /// Source of time for the record.
    pub clock: &'a dyn Clock,
    /// Captured on the first line of `main`.
    pub start: Instant,
}

fn elapsed_us(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// Records the execution when dropped, unless the thread is already unwinding (a second panic
/// would abort the process): the runtime catches every panic first, so that does not happen.
struct RecordOnDrop<'a> {
    events: Option<&'a dyn HookEventRepo>,
    clock: &'a dyn Clock,
    hook: &'static str,
    start: Instant,
    outcome: Outcome,
    note: Option<EventNote>,
}

impl Drop for RecordOnDrop<'_> {
    fn drop(&mut self) {
        let Some(events) = self.events else { return };
        if std::thread::panicking() || (self.note.is_none() && self.outcome == Outcome::Ok) {
            return;
        }
        let record = event_record(
            self.clock,
            self.hook,
            self.start,
            self.outcome,
            self.note.take(),
        );
        // A failed write is ignored (fail-open); so is a panic inside the repository.
        let _ = catch_unwind(AssertUnwindSafe(|| events.insert_event(&record)));
    }
}

fn event_record(
    clock: &dyn Clock,
    hook: &'static str,
    start: Instant,
    outcome: Outcome,
    note: Option<EventNote>,
) -> HookEventRecord {
    let failed = outcome != Outcome::Ok;
    let note = note.unwrap_or_else(|| EventNote {
        session_id: String::new(),
        decision: "none",
        reason: outcome.as_str(),
        bytes_saved: 0,
    });
    HookEventRecord {
        ts: clock.now(),
        session_id: note.session_id,
        hook: hook.to_owned(),
        decision: note.decision.to_owned(),
        reason: if failed {
            outcome.as_str()
        } else {
            note.reason
        }
        .to_owned(),
        bytes_saved: note.bytes_saved,
        duration_us: elapsed_us(start),
        outcome,
    }
}

/// Records a run that the time guard cut short (`outcome = timeout_guard`).
pub fn record_timeout(
    events: &dyn HookEventRepo,
    clock: &dyn Clock,
    event: HookEvent,
    start: Instant,
) {
    let record = event_record(clock, event.arg(), start, Outcome::TimeoutGuard, None);
    let _ = catch_unwind(AssertUnwindSafe(|| events.insert_event(&record)));
}

/// Runs `tokred hook <name>`: never fails and never panics.
///
/// `name` is the argument after `hook`; unknown or missing names do nothing.
pub fn run_hook(
    name: Option<&str>,
    stdin: &mut dyn Read,
    stdout: &mut dyn Write,
    run: &HookRun<'_>,
) {
    let Some(event) = name.and_then(HookEvent::from_arg) else {
        return;
    };
    let mut guard = RecordOnDrop {
        events: run.events,
        clock: run.clock,
        hook: event.arg(),
        start: run.start,
        outcome: Outcome::Ok,
        note: None,
    };

    let mut payload = Vec::new();
    if let Err(err) = Read::take(&mut *stdin, MAX_STDIN_BYTES).read_to_end(&mut payload) {
        tracing::error!(hook = event.arg(), error = %err, "could not read stdin (fail-open)");
        guard.outcome = Outcome::Error;
        return;
    }
    let Some(deps) = run.deps else {
        tracing::error!(hook = event.arg(), "no store available (fail-open)");
        return;
    };

    match catch_unwind(AssertUnwindSafe(|| handle(event, &payload, deps))) {
        Ok(Ok(response)) => {
            if let Some(text) = &response.stdout {
                // Write errors are ignored: a hook must never fail.
                let _ = stdout.write_all(text.as_bytes());
                let _ = stdout.flush();
            }
            guard.note = response.note;
        }
        Ok(Err(err)) => {
            tracing::error!(hook = event.arg(), error = %err, "hook failed (fail-open)");
            guard.outcome = Outcome::Error;
        }
        Err(_) => {
            tracing::error!(hook = event.arg(), "hook panicked (fail-open)");
            guard.outcome = Outcome::Panic;
        }
    }
    // `guard` is dropped here: the record is the last thing that happens.
}
