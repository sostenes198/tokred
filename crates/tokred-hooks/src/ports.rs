//! What the handlers need from the machine and from the stores, as `&dyn` ports.

use std::io;
use std::path::Path;

use tokred_core::Clock;
use tokred_core::Config;
use tokred_core::cache::CacheRepo;
use tokred_core::session::{ClassificationRepo, FileReadRepo, SessionRepo};

/// Size and modification time of a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    /// Size in bytes.
    pub size: u64,
    /// Whether the path is a directory.
    pub is_dir: bool,
    /// Modification time, in seconds since the Unix epoch.
    pub mtime: i64,
}

/// Read-only questions about files. The real adapter is [`crate::fs::StdFileMeta`].
pub trait FileMeta {
    /// Stats a path; `Ok(None)` when it does not exist.
    fn stat(&self, path: &Path) -> io::Result<Option<FileStat>>;
    /// blake3 hash (hex) of the file content.
    fn hash(&self, path: &Path) -> io::Result<String>;
    /// Counts the files under `root`, giving up (and returning `cap`) once `cap` is reached.
    fn count_files(&self, root: &Path, cap: u64) -> io::Result<u64>;
    /// The last `max_bytes` bytes of a file; `Ok(None)` when it does not exist.
    fn read_tail(&self, path: &Path, max_bytes: u64) -> io::Result<Option<Vec<u8>>>;
}

/// Everything a handler may touch.
pub struct HookDeps<'a> {
    /// Source of time.
    pub clock: &'a dyn Clock,
    /// File questions.
    pub files: &'a dyn FileMeta,
    /// Sessions.
    pub sessions: &'a dyn SessionRepo,
    /// File reads.
    pub reads: &'a dyn FileReadRepo,
    /// Classifications.
    pub classes: &'a dyn ClassificationRepo,
    /// Tiered cache.
    pub cache: &'a dyn CacheRepo,
    /// `tokred.toml`.
    pub config: &'a Config,
}

/// What a handler produced.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct HookResponse {
    /// JSON to print on stdout, if any.
    pub stdout: Option<String>,
    /// What to record in `hook_events`, if anything.
    pub note: Option<EventNote>,
}

/// The decision part of a `hook_events` row (the rest is filled in by the runtime).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventNote {
    /// Session id.
    pub session_id: String,
    /// `allow`, `warn`, `deny`, `ask`, `none` or, for `route`, the task class.
    pub decision: &'static str,
    /// Why.
    pub reason: &'static str,
    /// Bytes of context avoided.
    pub bytes_saved: u64,
}

impl HookResponse {
    /// No output and nothing to record.
    #[must_use]
    pub fn silent() -> Self {
        Self::default()
    }

    /// No output, but record the decision.
    #[must_use]
    pub fn noted(
        session_id: &str,
        decision: &'static str,
        reason: &'static str,
        bytes_saved: u64,
        stdout: Option<String>,
    ) -> Self {
        Self {
            stdout,
            note: Some(EventNote {
                session_id: session_id.to_owned(),
                decision,
                reason,
                bytes_saved,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silent_response_has_no_output_and_no_note() {
        assert_eq!(HookResponse::silent(), HookResponse::default());
        assert!(HookResponse::silent().note.is_none());
    }

    #[test]
    fn a_noted_response_carries_the_decision() {
        let r = HookResponse::noted("s", "deny", "reread", 5, Some("{}".into()));
        let note = r.note.unwrap();
        assert_eq!(
            (note.session_id.as_str(), note.decision, note.bytes_saved),
            ("s", "deny", 5)
        );
        assert_eq!(r.stdout.as_deref(), Some("{}"));
    }
}
