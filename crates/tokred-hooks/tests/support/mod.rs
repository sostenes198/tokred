//! Shared fakes for the hook tests: an in-memory file system, a movable clock and a harness
//! that wires them to a real (in-memory) SQLite store.
#![allow(
    dead_code,
    clippy::cast_possible_truncation,
    clippy::needless_pass_by_value,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tokred_core::{Clock, Config, HookEvent, Timestamp};
use tokred_hooks::{FileMeta, FileStat, HookDeps, HookResponse, handle};
use tokred_store::Store;

/// A clock that only moves when told to.
#[derive(Debug)]
pub struct FixedClock(pub Cell<i64>);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0.get())
    }
}

#[derive(Debug, Clone)]
struct Node {
    content: Vec<u8>,
    is_dir: bool,
    mtime: i64,
}

/// An in-memory [`FileMeta`] with call counters and failure switches.
#[derive(Debug, Default)]
pub struct FakeMeta {
    nodes: RefCell<HashMap<PathBuf, Node>>,
    /// Number returned by `count_files` (capped like the real one).
    pub file_count: Cell<u64>,
    /// How many times `count_files` ran.
    pub count_calls: Cell<u32>,
    /// How many times `hash` ran.
    pub hash_calls: Cell<u32>,
    /// Make `stat` fail.
    pub fail_stat: Cell<bool>,
    /// Make `hash` fail.
    pub fail_hash: Cell<bool>,
    /// Make `count_files` fail.
    pub fail_count: Cell<bool>,
    /// Make `read_tail` fail.
    pub fail_tail: Cell<bool>,
    /// Make `stat` panic.
    pub panic_stat: Cell<bool>,
    /// Sizes requested from `read_tail`, in order.
    pub tail_requests: RefCell<Vec<u64>>,
}

impl FakeMeta {
    /// Adds (or replaces) a file.
    pub fn file(&self, path: &str, content: &[u8]) {
        self.nodes.borrow_mut().insert(
            PathBuf::from(path),
            Node {
                content: content.to_vec(),
                is_dir: false,
                mtime: 1000,
            },
        );
    }

    /// Adds a directory.
    pub fn dir(&self, path: &str, mtime: i64) {
        self.nodes.borrow_mut().insert(
            PathBuf::from(path),
            Node {
                content: Vec::new(),
                is_dir: true,
                mtime,
            },
        );
    }

    /// Adds a file of `size` bytes.
    pub fn sized(&self, path: &str, size: usize) {
        self.file(path, &vec![b'x'; size]);
    }
}

impl FileMeta for FakeMeta {
    fn stat(&self, path: &Path) -> io::Result<Option<FileStat>> {
        assert!(!self.panic_stat.get(), "boom in stat");
        if self.fail_stat.get() {
            return Err(io::Error::other("stat failed"));
        }
        Ok(self.nodes.borrow().get(path).map(|n| FileStat {
            size: n.content.len() as u64,
            is_dir: n.is_dir,
            mtime: n.mtime,
        }))
    }

    fn hash(&self, path: &Path) -> io::Result<String> {
        self.hash_calls.set(self.hash_calls.get() + 1);
        if self.fail_hash.get() {
            return Err(io::Error::other("hash failed"));
        }
        let nodes = self.nodes.borrow();
        let node = nodes
            .get(path)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        Ok(blake3::hash(&node.content).to_hex().to_string())
    }

    fn count_files(&self, _root: &Path, cap: u64) -> io::Result<u64> {
        self.count_calls.set(self.count_calls.get() + 1);
        if self.fail_count.get() {
            return Err(io::Error::other("count failed"));
        }
        Ok(self.file_count.get().min(cap))
    }

    fn read_tail(&self, path: &Path, max_bytes: u64) -> io::Result<Option<Vec<u8>>> {
        self.tail_requests.borrow_mut().push(max_bytes);
        if self.fail_tail.get() {
            return Err(io::Error::other("tail failed"));
        }
        Ok(self.nodes.borrow().get(path).map(|n| {
            let start = n.content.len().saturating_sub(max_bytes as usize);
            n.content[start..].to_vec()
        }))
    }
}

/// Fake file system + clock + config around a real in-memory store.
pub struct Env {
    pub store: Store,
    pub files: FakeMeta,
    pub clock: FixedClock,
    pub config: Config,
}

impl Env {
    pub fn new() -> Self {
        Self {
            store: Store::open_in_memory().unwrap(),
            files: FakeMeta::default(),
            clock: FixedClock(Cell::new(1_000_000)),
            config: Config::default(),
        }
    }

    pub fn deps(&self) -> HookDeps<'_> {
        HookDeps {
            clock: &self.clock,
            files: &self.files,
            sessions: &self.store,
            reads: &self.store,
            classes: &self.store,
            cache: &self.store,
            config: &self.config,
        }
    }

    pub fn advance(&self, seconds: i64) {
        self.clock.0.set(self.clock.0.get() + seconds);
    }

    /// Runs a hook with a JSON payload.
    pub fn run(&self, event: HookEvent, payload: &Value) -> HookResponse {
        handle(event, payload.to_string().as_bytes(), &self.deps()).unwrap()
    }
}

pub const CWD: &str = "/work/proj";

/// A `PreToolUse` payload.
pub fn tool(session: &str, name: &str, input: Value) -> Value {
    json!({
        "session_id": session,
        "cwd": CWD,
        "hook_event_name": "PreToolUse",
        "tool_name": name,
        "tool_input": input,
    })
}

/// The `permissionDecision` of an answer, if it is a denial.
pub fn denial(response: &HookResponse) -> Option<String> {
    let value: Value = serde_json::from_str(response.stdout.as_deref()?).ok()?;
    let out = value.get("hookSpecificOutput")?;
    (out.get("permissionDecision")? == "deny")
        .then(|| out["permissionDecisionReason"].as_str().unwrap().to_owned())
}

/// The `additionalContext` of an answer.
pub fn context_of(response: &HookResponse) -> Option<String> {
    let value: Value = serde_json::from_str(response.stdout.as_deref()?).ok()?;
    Some(
        value["hookSpecificOutput"]["additionalContext"]
            .as_str()?
            .to_owned(),
    )
}

/// `(decision, reason, bytes_saved)` of the recorded note.
pub fn note(response: &HookResponse) -> (String, String, u64) {
    let note = response.note.clone().expect("a note was recorded");
    (
        note.decision.to_owned(),
        note.reason.to_owned(),
        note.bytes_saved,
    )
}
