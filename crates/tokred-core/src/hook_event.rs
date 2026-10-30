//! Records of hook executions (table `hook_events`) and the conventions the metrics rely on.
//!
//! * `hook` is the hook name (`guard-read`, `route`, ...), `decision` is what it decided and
//!   `reason` why. The vocabulary below is shared by the hooks that write the rows and the
//!   metrics that read them.

use crate::error::StoreError;
use crate::types::Timestamp;

/// How a hook execution ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Finished normally.
    Ok,
    /// Failed with an error (fail-open: nothing was blocked).
    Error,
    /// Panicked (fail-open).
    Panic,
    /// Stopped by the internal time guard (fail-open).
    TimeoutGuard,
}

impl Outcome {
    /// Value stored in the `outcome` column.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Panic => "panic",
            Self::TimeoutGuard => "timeout_guard",
        }
    }

    /// Parses the stored value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        [Self::Ok, Self::Error, Self::Panic, Self::TimeoutGuard]
            .into_iter()
            .find(|outcome| outcome.as_str() == value)
    }
}

/// `decision` values.
pub mod decision {
    /// The hook let the action through.
    pub const ALLOW: &str = "allow";
    /// The hook let the action through but advised against it.
    pub const WARN: &str = "warn";
    /// The hook denied the action.
    pub const DENY: &str = "deny";
    /// The hook asked the user to confirm the action.
    pub const ASK: &str = "ask";
    /// The hook did nothing (mode `off`, or nothing to say).
    pub const NONE: &str = "none";
}

/// `reason` values used by the guards (shown in the "Guards" section of the report).
pub mod reason {
    /// File unchanged since it was read in this session.
    pub const REREAD: &str = "reread";
    /// Lockfile, minified, build output and the like.
    pub const GENERATED_FILE: &str = "generated_file";
    /// Larger than `max_read_bytes` without `offset`/`limit`.
    pub const LARGE_FILE: &str = "large_file";
    /// Search without `path` in a large repository.
    pub const BROAD_SEARCH: &str = "broad_search";
    /// `enforce`: the chosen subagent matches the classification.
    pub const AGENT_MATCHES: &str = "agent_matches";
    /// `enforce`: the chosen subagent differs from the classification.
    pub const AGENT_MISMATCH: &str = "agent_mismatch";
    /// `route`: a suggestion was emitted (the `decision` column holds the task class).
    pub const SUGGESTED: &str = "suggested";
    /// `route`: the class did not change, or the prompt was too small to delegate.
    pub const UNCHANGED: &str = "unchanged";
}

/// One hook execution (row of `hook_events`).
///
/// For `route` the `decision` column holds the task class (`architecture`, `planning`,
/// `implementation`, `review`, `search`, `summary`, `other`) so the metrics can count
/// classifications without a table of their own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookEventRecord {
    /// When the hook ran.
    pub ts: Timestamp,
    /// Session id (empty if unknown).
    pub session_id: String,
    /// Hook name.
    pub hook: String,
    /// What it decided.
    pub decision: String,
    /// Why.
    pub reason: String,
    /// Bytes of context avoided (guards only, else 0).
    pub bytes_saved: u64,
    /// Time spent inside the tokred process, in microseconds.
    pub duration_us: u64,
    /// How it ended.
    pub outcome: Outcome,
}

/// Persistent hook executions.
pub trait HookEventRepo {
    /// Stores one execution.
    fn insert_event(&self, event: &HookEventRecord) -> Result<(), StoreError>;
    /// Executions with `from <= ts < to`, ordered by `ts`.
    fn events_between(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<HookEventRecord>, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_round_trip() {
        for outcome in [
            Outcome::Ok,
            Outcome::Error,
            Outcome::Panic,
            Outcome::TimeoutGuard,
        ] {
            assert_eq!(Outcome::parse(outcome.as_str()), Some(outcome));
        }
        assert_eq!(Outcome::parse("nope"), None);
    }
}
