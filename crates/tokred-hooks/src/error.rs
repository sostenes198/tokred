//! Errors of the hook handlers. They never reach Claude Code: the runtime turns every one of
//! them into "exit 0, no output".

use tokred_core::StoreError;

/// Why a hook could not decide.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    /// Stdin was not the JSON object Claude Code documents.
    #[error("invalid hook input: {0}")]
    Input(#[from] serde_json::Error),
    /// The local store failed.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    /// Reading stdin or a file failed.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_readable() {
        let input = HookError::from(serde_json::from_str::<u8>("x").unwrap_err());
        assert!(input.to_string().contains("invalid hook input"));
        let store = HookError::from(StoreError::backend(std::io::Error::other("disk")));
        assert!(store.to_string().contains("disk"));
        let io = HookError::from(std::io::Error::other("pipe"));
        assert!(io.to_string().contains("pipe"));
    }
}
