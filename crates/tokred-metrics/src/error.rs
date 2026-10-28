//! Errors of the metrics use cases.

use tokred_core::{FsError, StoreError};

/// Failure while ingesting transcripts.
#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    /// A transcript could not be listed or read.
    #[error(transparent)]
    Fs(#[from] FsError),
    /// The database rejected a read or write.
    #[error(transparent)]
    Store(#[from] StoreError),
}
