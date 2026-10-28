//! Release information (for the daily version check and self-update).

use crate::pricing::SourceError;

/// The latest published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Version without a leading `v` (`0.4.0`).
    pub version: String,
    /// Page of the release.
    pub url: String,
}

/// Remote source of release information (network adapter; never used inside a hook).
pub trait ReleaseSource {
    /// Fetches the latest stable release.
    fn latest(&self) -> Result<Release, SourceError>;
}
