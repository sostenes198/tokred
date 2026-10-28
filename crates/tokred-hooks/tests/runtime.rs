//! The runtime: fail-open behaviour, ordering of the answer and the record, and outcomes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation
)]

mod support;

use std::cell::RefCell;
use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::json;
use support::{CWD, Env, tool};
use tokred_core::config::Mode;
use tokred_core::hook_event::{HookEventRecord, HookEventRepo, Outcome};
use tokred_core::{StoreError, Timestamp};
use tokred_hooks::{HookRun, record_timeout, run_hook};

fn events(env: &Env) -> Vec<HookEventRecord> {
    env.store
        .events_between(Timestamp(0), Timestamp(i64::MAX))
        .unwrap()
}

fn run(env: &Env, name: Option<&str>, stdin: &[u8]) -> (String, Vec<HookEventRecord>) {
    let deps = env.deps();
    let mut out = Vec::new();
    let ctx = HookRun {
        deps: Some(&deps),
        events: Some(&env.store),
        clock: &env.clock,
        start: Instant::now(),
    };
    run_hook(name, &mut &*stdin, &mut out, &ctx);
    (String::from_utf8(out).unwrap(), events(env))
}

fn lockfile_read() -> String {
    tool("s", "Read", json!({"file_path": "/work/proj/Cargo.lock"})).to_string()
}

#[test]
fn a_denial_is_written_and_recorded_with_its_duration() {
    let env = Env::new();
    env.files.sized("/work/proj/Cargo.lock", 500);
    let (stdout, recorded) = run(&env, Some("guard-read"), lockfile_read().as_bytes());
    assert!(
        stdout.contains("\"permissionDecision\":\"deny\""),
        "{stdout}"
    );
    assert_eq!(recorded.len(), 1);
    let event = &recorded[0];
    assert_eq!(event.hook, "guard-read");
    assert_eq!(event.session_id, "s");
    assert_eq!(
        (event.decision.as_str(), event.reason.as_str()),
        ("deny", "generated_file")
    );
    assert_eq!((event.bytes_saved, event.outcome), (500, Outcome::Ok));
    assert_eq!(event.ts, Timestamp(1_000_000));
}

#[test]
fn the_record_is_the_last_step_after_the_answer_was_written_and_flushed() {
    struct Log(Rc<RefCell<Vec<&'static str>>>);
    impl Write for Log {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().push("write");
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.borrow_mut().push("flush");
            Ok(())
        }
    }
    struct LoggingRepo(Rc<RefCell<Vec<&'static str>>>);
    impl HookEventRepo for LoggingRepo {
        fn insert_event(&self, _: &HookEventRecord) -> Result<(), StoreError> {
            self.0.borrow_mut().push("record");
            Ok(())
        }
        fn events_between(
            &self,
            _: Timestamp,
            _: Timestamp,
        ) -> Result<Vec<HookEventRecord>, StoreError> {
            Ok(vec![])
        }
    }

    let env = Env::new();
    env.files.sized("/work/proj/Cargo.lock", 5);
    let log = Rc::new(RefCell::new(Vec::new()));
    let repo = LoggingRepo(Rc::clone(&log));
    let deps = env.deps();
    let ctx = HookRun {
        deps: Some(&deps),
        events: Some(&repo),
        clock: &env.clock,
        start: Instant::now(),
    };
    run_hook(
        Some("guard-read"),
        &mut lockfile_read().as_bytes(),
        &mut Log(Rc::clone(&log)),
        &ctx,
    );
    assert_eq!(*log.borrow(), ["write", "flush", "record"]);
}

#[test]
fn garbage_stdin_is_an_error_outcome_with_no_output() {
    let env = Env::new();
    for garbage in [&b"\0not json"[..], b"", b"[1,2]"] {
        let (stdout, _) = run(&env, Some("route"), garbage);
        assert_eq!(stdout, "");
    }
    let recorded = events(&env);
    assert_eq!(recorded.len(), 3);
    for event in recorded {
        assert_eq!(event.outcome, Outcome::Error);
        assert_eq!(
            (event.decision.as_str(), event.reason.as_str()),
            ("none", "error")
        );
        assert_eq!(event.hook, "route");
        assert_eq!(event.session_id, "");
    }
}

#[test]
fn a_panic_in_a_handler_is_caught_and_recorded() {
    let env = Env::new();
    env.files.panic_stat.set(true);
    let (stdout, recorded) = run(&env, Some("guard-read"), lockfile_read().as_bytes());
    assert_eq!(stdout, "");
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].outcome, Outcome::Panic);
    assert_eq!(recorded[0].reason, "panic");
}

#[test]
fn a_store_error_in_a_handler_is_an_error_outcome() {
    let env = Env::new();
    env.files.fail_stat.set(true);
    let (stdout, recorded) = run(&env, Some("guard-read"), lockfile_read().as_bytes());
    assert_eq!(stdout, "");
    assert_eq!(recorded[0].outcome, Outcome::Error);
}

#[test]
fn unknown_or_missing_hook_names_do_nothing_at_all() {
    let env = Env::new();
    for name in [None, Some("from-the-future"), Some("")] {
        let (stdout, recorded) = run(&env, name, lockfile_read().as_bytes());
        assert_eq!(stdout, "");
        assert!(recorded.is_empty());
    }
}

#[test]
fn hooks_with_nothing_to_say_record_nothing() {
    let mut env = Env::new();
    env.config.modes.guard_read = Mode::Off;
    env.files.sized("/work/proj/Cargo.lock", 5);
    let (stdout, recorded) = run(&env, Some("guard-read"), lockfile_read().as_bytes());
    assert_eq!(stdout, "");
    assert!(recorded.is_empty());
}

#[test]
fn an_allowed_call_prints_nothing_and_is_still_recorded() {
    let env = Env::new();
    env.files.file("/work/proj/a.rs", b"x");
    let payload = tool("s", "Read", json!({"file_path": "/work/proj/a.rs"})).to_string();
    let (stdout, recorded) = run(&env, Some("guard-read"), payload.as_bytes());
    assert_eq!(stdout, "");
    assert_eq!(recorded[0].decision, "allow");
}

#[test]
fn without_a_store_nothing_happens() {
    let env = Env::new();
    let ctx = HookRun {
        deps: None,
        events: None,
        clock: &env.clock,
        start: Instant::now(),
    };
    let mut out = Vec::new();
    run_hook(
        Some("guard-read"),
        &mut lockfile_read().as_bytes(),
        &mut out,
        &ctx,
    );
    assert!(out.is_empty());
}

#[test]
fn an_unreadable_stdin_is_an_error_outcome() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("closed"))
        }
    }
    let env = Env::new();
    let deps = env.deps();
    let ctx = HookRun {
        deps: Some(&deps),
        events: Some(&env.store),
        clock: &env.clock,
        start: Instant::now(),
    };
    let mut out = Vec::new();
    run_hook(Some("route"), &mut Broken, &mut out, &ctx);
    assert!(out.is_empty());
    assert_eq!(events(&env)[0].outcome, Outcome::Error);
}

#[test]
fn stdin_is_capped() {
    struct Endless;
    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            buf.fill(b' ');
            Ok(buf.len())
        }
    }
    let env = Env::new();
    let deps = env.deps();
    let ctx = HookRun {
        deps: Some(&deps),
        events: Some(&env.store),
        clock: &env.clock,
        start: Instant::now(),
    };
    let mut out = Vec::new();
    run_hook(Some("route"), &mut Endless, &mut out, &ctx);
    // Blank input is not a JSON object: an error, but the read terminated.
    assert_eq!(events(&env)[0].outcome, Outcome::Error);
}

#[test]
fn a_closed_stdout_is_ignored() {
    struct Closed;
    impl Write for Closed {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("closed pipe"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("closed pipe"))
        }
    }
    let env = Env::new();
    env.files.sized("/work/proj/Cargo.lock", 5);
    let deps = env.deps();
    let ctx = HookRun {
        deps: Some(&deps),
        events: Some(&env.store),
        clock: &env.clock,
        start: Instant::now(),
    };
    run_hook(
        Some("guard-read"),
        &mut lockfile_read().as_bytes(),
        &mut Closed,
        &ctx,
    );
    assert_eq!(events(&env).len(), 1);
}

struct FailingRepo;
impl HookEventRepo for FailingRepo {
    fn insert_event(&self, _: &HookEventRecord) -> Result<(), StoreError> {
        Err(StoreError::backend(std::io::Error::other("disk full")))
    }
    fn events_between(
        &self,
        _: Timestamp,
        _: Timestamp,
    ) -> Result<Vec<HookEventRecord>, StoreError> {
        Ok(vec![])
    }
}

struct PanickingRepo;
impl HookEventRepo for PanickingRepo {
    fn insert_event(&self, _: &HookEventRecord) -> Result<(), StoreError> {
        panic!("boom in the repository");
    }
    fn events_between(
        &self,
        _: Timestamp,
        _: Timestamp,
    ) -> Result<Vec<HookEventRecord>, StoreError> {
        Ok(vec![])
    }
}

#[test]
fn a_failing_or_panicking_record_write_is_ignored_and_the_answer_survives() {
    let env = Env::new();
    env.files.sized("/work/proj/Cargo.lock", 5);
    let deps = env.deps();
    for repo in [&FailingRepo as &dyn HookEventRepo, &PanickingRepo] {
        let ctx = HookRun {
            deps: Some(&deps),
            events: Some(repo),
            clock: &env.clock,
            start: Instant::now(),
        };
        let mut out = Vec::new();
        run_hook(
            Some("guard-read"),
            &mut lockfile_read().as_bytes(),
            &mut out,
            &ctx,
        );
        assert!(String::from_utf8(out).unwrap().contains("deny"));
    }
    record_timeout(
        &FailingRepo,
        &env.clock,
        tokred_core::HookEvent::Route,
        Instant::now(),
    );
    record_timeout(
        &PanickingRepo,
        &env.clock,
        tokred_core::HookEvent::Route,
        Instant::now(),
    );
}

#[test]
fn a_panic_outside_the_handler_does_not_abort_while_recording() {
    // A panicking stdout unwinds through `run_hook` while the record guard is alive: the guard
    // must not write (a second panic would abort the process).
    struct PanicWriter;
    impl Write for PanicWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            panic!("boom in stdout");
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let env = Env::new();
    env.files.sized("/work/proj/Cargo.lock", 5);
    let deps = env.deps();
    let ctx = HookRun {
        deps: Some(&deps),
        events: Some(&env.store),
        clock: &env.clock,
        start: Instant::now(),
    };
    let result = catch_unwind(AssertUnwindSafe(|| {
        run_hook(
            Some("guard-read"),
            &mut lockfile_read().as_bytes(),
            &mut PanicWriter,
            &ctx,
        );
    }));
    assert!(result.is_err());
    assert!(events(&env).is_empty());
}

#[test]
fn a_timeout_is_recorded_as_timeout_guard() {
    let env = Env::new();
    let started = Instant::now()
        .checked_sub(Duration::from_millis(5))
        .unwrap();
    record_timeout(
        &env.store,
        &env.clock,
        tokred_core::HookEvent::GuardSearch,
        started,
    );
    let recorded = events(&env);
    assert_eq!(recorded[0].outcome, Outcome::TimeoutGuard);
    assert_eq!(recorded[0].hook, "guard-search");
    assert_eq!(recorded[0].reason, "timeout_guard");
    assert!(recorded[0].duration_us >= 5000);
}

#[test]
fn every_hook_runs_on_a_realistic_fixture_without_failing() {
    let env = Env::new();
    for (event, stdin) in tokred_hooks::bench_fixtures() {
        let (_, _) = run(&env, Some(event.arg()), stdin.as_bytes());
    }
    let recorded = events(&env);
    assert!(!recorded.is_empty());
    assert!(
        recorded.iter().all(|e| e.outcome == Outcome::Ok),
        "{recorded:?}"
    );
    let _ = CWD;
}
