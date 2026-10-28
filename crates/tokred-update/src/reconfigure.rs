//! Reapplying the configuration after the binary changed version (spec section 14).

use tokred_core::error::StoreError;
use tokred_core::meta::MetaRepo;

/// Meta key: the tokred version that last configured Claude Code.
pub const TOKRED_VERSION: &str = "tokred_version";

/// Whether hooks and agents must be reapplied: tokred was configured by a different version than
/// the one running now. A machine that was never configured (no recorded version) does not need
/// it: installing and configuring are separate steps, and `configure` is the user's call. An
/// unreadable meta store also answers `false`.
#[must_use]
pub fn needs_reconfigure(meta: &dyn MetaRepo, current_version: &str) -> bool {
    matches!(meta.meta_get(TOKRED_VERSION), Ok(Some(recorded)) if recorded != current_version)
}

/// Records that `version` has configured Claude Code (call it after `configure`).
pub fn mark_configured(meta: &dyn MetaRepo, version: &str) -> Result<(), StoreError> {
    meta.meta_set(TOKRED_VERSION, version)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct FakeMeta {
        map: RefCell<HashMap<String, String>>,
        broken: bool,
    }

    impl MetaRepo for FakeMeta {
        fn meta_get(&self, key: &str) -> Result<Option<String>, StoreError> {
            if self.broken {
                return Err(StoreError::backend(std::io::Error::other("down")));
            }
            Ok(self.map.borrow().get(key).cloned())
        }

        fn meta_set(&self, key: &str, value: &str) -> Result<(), StoreError> {
            if self.broken {
                return Err(StoreError::backend(std::io::Error::other("down")));
            }
            self.map
                .borrow_mut()
                .insert(key.to_owned(), value.to_owned());
            Ok(())
        }
    }

    #[test]
    fn reconfigure_is_needed_only_after_a_version_change() {
        let meta = FakeMeta::default();
        assert!(!needs_reconfigure(&meta, "0.4.0"), "never configured");
        mark_configured(&meta, "0.3.2").unwrap();
        assert!(needs_reconfigure(&meta, "0.4.0"));
        assert!(needs_reconfigure(&meta, "0.3.1"), "downgrades too");
        mark_configured(&meta, "0.4.0").unwrap();
        assert!(!needs_reconfigure(&meta, "0.4.0"));
        assert_eq!(
            meta.meta_get(TOKRED_VERSION).unwrap().as_deref(),
            Some("0.4.0")
        );
    }

    #[test]
    fn a_broken_store_is_reported_on_write_and_ignored_on_read() {
        let meta = FakeMeta {
            broken: true,
            ..FakeMeta::default()
        };
        assert!(!needs_reconfigure(&meta, "0.4.0"));
        assert!(mark_configured(&meta, "0.4.0").is_err());
    }
}
