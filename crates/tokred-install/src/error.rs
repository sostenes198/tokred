//! Errors of the install use cases.

use std::path::PathBuf;

use tokred_core::{FsError, StoreError};

/// The `settings.json` structure is not what Claude Code expects, so it cannot be merged safely.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MergeError {
    /// The document root is not a JSON object.
    #[error("settings root must be a JSON object")]
    RootNotObject,
    /// `hooks` exists but is not an object.
    #[error("`hooks` must be a JSON object")]
    HooksNotObject,
    /// An event under `hooks` is not an array of matcher groups.
    #[error("`hooks.{0}` must be an array")]
    EventNotArray(String),
}

/// Failure of a configure, unconfigure or doctor run.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    /// File system failure.
    #[error(transparent)]
    Fs(#[from] FsError),
    /// Store failure.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// `settings.json` is not valid JSON; nothing was written.
    #[error("`{}` is not valid JSON ({source}); nothing was changed", path.display())]
    InvalidJson {
        /// The offending file.
        path: PathBuf,
        /// Parse error.
        #[source]
        source: serde_json::Error,
    },
    /// `settings.json` has an unexpected structure; nothing was written.
    #[error("cannot merge `{}`: {source}; nothing was changed", path.display())]
    Merge {
        /// The offending file.
        path: PathBuf,
        /// Structural problem.
        #[source]
        source: MergeError,
    },
    /// The merged settings could not be serialized.
    #[error("could not serialize settings: {0}")]
    Serialize(#[from] serde_json::Error),
}
