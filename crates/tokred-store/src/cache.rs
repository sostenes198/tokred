//! Repository for the tiered cache (`cache_entries`).

use rusqlite::{OptionalExtension, params};
use tokred_core::cache::{CacheEntry, CacheRepo, Tier};
use tokred_core::{StoreError, Timestamp};

use crate::Store;

const COLUMNS: &str = "key, kind, value, tier, hits, created_at, last_hit, expires_at";

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<CacheEntry> {
    Ok(CacheEntry {
        key: row.get(0)?,
        kind: row.get(1)?,
        value: row.get(2)?,
        tier: Tier::from_int(row.get(3)?).unwrap_or(Tier::L1),
        hits: u32::try_from(row.get::<_, i64>(4)?).unwrap_or(u32::MAX),
        created_at: Timestamp(row.get(5)?),
        last_hit: Timestamp(row.get(6)?),
        expires_at: row.get::<_, Option<i64>>(7)?.map(Timestamp),
    })
}

impl CacheRepo for Store {
    fn get_entry(&self, key: &str) -> Result<Option<CacheEntry>, StoreError> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM cache_entries WHERE key = ?1"),
                params![key],
                row_to_entry,
            )
            .optional()
            .map_err(StoreError::backend)
    }

    fn put_entry(&self, entry: &CacheEntry) -> Result<(), StoreError> {
        self.conn
            .execute(
                &format!(
                    "INSERT OR REPLACE INTO cache_entries({COLUMNS}) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
                ),
                params![
                    entry.key,
                    entry.kind,
                    entry.value,
                    entry.tier.as_int(),
                    entry.hits,
                    entry.created_at.0,
                    entry.last_hit.0,
                    entry.expires_at.map(|at| at.0)
                ],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }

    fn delete_expired(&self, now: Timestamp) -> Result<usize, StoreError> {
        self.conn
            .execute(
                "DELETE FROM cache_entries WHERE expires_at IS NOT NULL AND expires_at <= ?1",
                params![now.0],
            )
            .map_err(StoreError::backend)
    }

    fn list_entries(&self) -> Result<Vec<CacheEntry>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {COLUMNS} FROM cache_entries ORDER BY key"))
            .map_err(StoreError::backend)?;
        stmt.query_map([], row_to_entry)
            .map_err(StoreError::backend)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::backend)
    }
}
