//! Small key/value store for state that has no table of its own (table `meta`).
//!
//! Keys in use: `tokred_version` (version that last configured), `price_synced_at`,
//! `version_check_at`, `version_check_latest`, `bench_spawn_overhead_us`.

use crate::error::StoreError;

/// Persistent key/value pairs.
pub trait MetaRepo {
    /// Value of `key`.
    fn meta_get(&self, key: &str) -> Result<Option<String>, StoreError>;
    /// Sets `key`.
    fn meta_set(&self, key: &str, value: &str) -> Result<(), StoreError>;
}
