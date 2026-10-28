//! Plain domain types shared across crates.

use std::path::PathBuf;

/// A point in time, as whole seconds since the Unix epoch. Serializes as the bare number.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct Timestamp(pub i64);

/// A file installed by tokred into the Claude Code folder (row of `installed_files`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestEntry {
    /// Absolute path of the installed file.
    pub path: PathBuf,
    /// blake3 hash (hex) of the content tokred installed.
    pub hash: String,
    /// tokred version that installed the file.
    pub version: String,
    /// When the file was installed.
    pub installed_at: Timestamp,
}
