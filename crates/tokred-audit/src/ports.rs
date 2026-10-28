//! Ports owned by the audit: directory listing and MCP usage lookup.
//!
//! File reads go through the shared [`tokred_core::FileSystem`] port; the two traits here cover
//! what that port cannot express.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tokred_core::{FsError, Timestamp};

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Full path of the entry.
    pub path: PathBuf,
    /// Whether the entry is a directory (symlinks are followed).
    pub is_dir: bool,
}

/// Lists the entries of a directory.
pub trait DirLister {
    /// Lists `dir`; a directory that does not exist yields an empty list.
    fn list_dir(&self, dir: &Path) -> Result<Vec<DirEntry>, FsError>;
}

/// Usage of one MCP server inside a time window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct McpUsage {
    /// Number of `tool_use` calls.
    pub calls: u64,
    /// Time of the most recent call in the window.
    pub last_seen: Option<Timestamp>,
}

/// Result of scanning the transcripts for MCP calls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageScan {
    /// Usage per server, keyed by the server segment of the tool name (`mcp__<server>__<tool>`).
    pub servers: BTreeMap<String, McpUsage>,
    /// Number of transcript files that were read (files outside the window are not counted).
    pub files_scanned: usize,
}

/// Failure to look up MCP usage.
#[derive(Debug, thiserror::Error)]
pub enum UsageError {
    /// The transcript location exists but cannot be scanned.
    #[error("transcripts at `{}` cannot be scanned: {reason}", path.display())]
    Unavailable {
        /// Transcript root that was being scanned.
        path: PathBuf,
        /// Why it cannot be scanned.
        reason: String,
    },
}

/// Source of MCP usage counts.
pub trait McpUsageSource {
    /// Counts `tool_use` calls to `mcp__<server>__*` tools made at or after `since`.
    fn usage_since(&self, since: Timestamp) -> Result<UsageScan, UsageError>;
}
