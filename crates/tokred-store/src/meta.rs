//! `meta` key/value repository.

use rusqlite::{OptionalExtension, params};
use tokred_core::StoreError;
use tokred_core::meta::MetaRepo;

use crate::Store;

impl MetaRepo for Store {
    fn meta_get(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::backend)
    }

    fn meta_set(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO meta(key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }
}
