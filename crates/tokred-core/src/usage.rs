//! Token usage extracted from Claude Code transcripts, and its persistence ports.

use crate::error::StoreError;
use crate::pricing::Usage;
use crate::types::Timestamp;

/// One assistant message (row of `usage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageRecord {
    /// Deduplication key: message id + request id (streaming can repeat lines).
    pub message_key: String,
    /// Session id.
    pub session_id: String,
    /// Project folder the transcript belongs to.
    pub project: String,
    /// When the message was produced.
    pub ts: Timestamp,
    /// Model id.
    pub model: String,
    /// Token counts.
    pub usage: Usage,
    /// The line came from a subagent transcript.
    pub is_subagent: bool,
    /// Who sent the request (`main`, `subagent`, `auxiliary`, `compaction`). Known only for rows
    /// the proxy measured; `None` for rows read from transcripts alone, and never stored.
    pub origin: Option<String>,
    /// The agent type of a subagent request, when the proxy saw it. Never stored.
    pub agent_type: Option<String>,
}

/// Incremental-ingestion position of one transcript file (row of `ingest_state`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestState {
    /// Transcript path.
    pub file: String,
    /// Byte offset already ingested.
    pub offset: u64,
    /// File modification time (unix seconds) when it was last read.
    pub mtime: i64,
}

/// Persistent usage rows.
pub trait UsageRepo {
    /// Inserts rows whose `message_key` is new; returns how many were inserted.
    fn insert_new(&self, records: &[UsageRecord]) -> Result<usize, StoreError>;
    /// Rows with `from <= ts < to`, ordered by `ts`.
    fn usage_between(&self, from: Timestamp, to: Timestamp)
    -> Result<Vec<UsageRecord>, StoreError>;
    /// Timestamp of the oldest row, if any.
    fn earliest_usage(&self) -> Result<Option<Timestamp>, StoreError>;
}

/// Persistent ingestion positions.
pub trait IngestRepo {
    /// Position of `file`, if it was ingested before.
    fn ingest_state(&self, file: &str) -> Result<Option<IngestState>, StoreError>;
    /// Saves a position.
    fn save_ingest_state(&self, state: &IngestState) -> Result<(), StoreError>;
}
