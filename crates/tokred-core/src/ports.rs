//! Ports: the traits through which use cases reach the outside world.

use std::path::{Path, PathBuf};

use crate::error::{FsError, StoreError};
use crate::types::{ManifestEntry, Timestamp};

/// Source of the current time.
pub trait Clock {
    /// Returns the current time.
    fn now(&self) -> Timestamp;
}

/// The subset of file system operations the use cases need.
pub trait FileSystem {
    /// Reads a UTF-8 file; `Ok(None)` when it does not exist.
    fn read_to_string(&self, path: &Path) -> Result<Option<String>, FsError>;
    /// Writes a file atomically (temporary file in the same directory, then rename).
    fn write_atomic(&self, path: &Path, contents: &[u8]) -> Result<(), FsError>;
    /// Removes a file; returns whether it existed.
    fn remove_file(&self, path: &Path) -> Result<bool, FsError>;
    /// Removes a directory and everything under it; returns whether it existed.
    fn remove_dir_all(&self, path: &Path) -> Result<bool, FsError>;
    /// Creates a directory and its parents.
    fn create_dir_all(&self, path: &Path) -> Result<(), FsError>;
    /// Whether the path exists.
    fn exists(&self, path: &Path) -> bool;
    /// Whether the path is an existing directory.
    fn is_dir(&self, path: &Path) -> bool;
}

/// Repository of files installed by tokred (`installed_files`).
pub trait ManifestRepo {
    /// Looks up the entry for `path`.
    fn get(&self, path: &Path) -> Result<Option<ManifestEntry>, StoreError>;
    /// Inserts or replaces an entry.
    fn upsert(&self, entry: &ManifestEntry) -> Result<(), StoreError>;
    /// Deletes the entry for `path`.
    fn remove(&self, path: &Path) -> Result<(), StoreError>;
    /// Lists all entries.
    fn list(&self) -> Result<Vec<ManifestEntry>, StoreError>;
}

/// Convenience alias for a list of paths.
pub type PathList = Vec<PathBuf>;
