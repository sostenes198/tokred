//! Error enums used by the ports and the configuration model.

use std::path::PathBuf;

/// Failure of a [`crate::ports::FileSystem`] operation.
#[derive(Debug, thiserror::Error)]
#[error("file system error on `{}`: {source}", path.display())]
pub struct FsError {
    /// Path the operation was acting on.
    pub path: PathBuf,
    /// Underlying I/O error.
    #[source]
    pub source: std::io::Error,
}

impl FsError {
    /// Builds an [`FsError`] for `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self {
            path: path.into(),
            source,
        }
    }
}

/// Failure of a repository backed by the local store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The storage backend reported an error.
    #[error("store backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl StoreError {
    /// Wraps a backend error.
    pub fn backend(err: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Backend(Box::new(err))
    }
}

/// Failure while reading or merging `tokred.toml`.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file is not valid TOML.
    #[error("invalid TOML in tokred.toml: {0}")]
    Parse(#[from] toml::de::Error),
    /// The merged configuration could not be serialized.
    #[error("could not serialize tokred.toml: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// A section that must be a table has another type.
    #[error("`{0}` in tokred.toml must be a table")]
    NotATable(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fs_error_names_the_path_and_keeps_the_source() {
        let err = FsError::new("/a/b", std::io::Error::other("boom"));
        assert_eq!(err.path, PathBuf::from("/a/b"));
        assert!(err.to_string().contains("/a/b") && err.to_string().contains("boom"));
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn store_error_wraps_the_backend_error() {
        let err = StoreError::backend(std::io::Error::other("disk"));
        assert!(err.to_string().contains("disk"));
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn config_errors_are_readable() {
        let parse = toml::from_str::<toml::Table>("[[[").unwrap_err();
        assert!(
            ConfigError::from(parse)
                .to_string()
                .contains("invalid TOML")
        );
        assert!(
            ConfigError::NotATable("modes".into())
                .to_string()
                .contains("modes")
        );
    }
}
