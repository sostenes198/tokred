//! `stored_results`, `compressed_blocks` and `shadow_savings` repositories (reversible
//! compression and shadow mode, spec 6.6 and 6.7).

use rusqlite::{OptionalExtension, params};
use tokred_core::compression::{CompressedBlock, ShadowRecord, ShadowSavings, StoredResult};
use tokred_core::{StoreError, Timestamp};

use crate::Store;

const STORED_COLUMNS: &str =
    "id, session_id, tool, original_hash, content, lines, bytes, created_at, expires_at";

const BLOCK_COLUMNS: &str = "session_id, original_hash, replacement, rules_version, ts";

/// SQLite integers are signed: counters beyond `i64::MAX` saturate (unreachable in practice).
fn to_db(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Counters are never negative; a corrupt negative value reads as zero.
fn from_db(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn row_to_stored(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredResult> {
    let content: Vec<u8> = row.get(4)?;
    Ok(StoredResult {
        id: row.get(0)?,
        session_id: row.get(1)?,
        tool: row.get(2)?,
        original_hash: row.get(3)?,
        // Written from a `String`, so always UTF-8; a damaged row still reads back.
        content: String::from_utf8_lossy(&content).into_owned(),
        lines: from_db(row.get(5)?),
        bytes: from_db(row.get(6)?),
        created_at: Timestamp(row.get(7)?),
        expires_at: Timestamp(row.get(8)?),
    })
}

fn row_to_block(row: &rusqlite::Row<'_>) -> rusqlite::Result<CompressedBlock> {
    Ok(CompressedBlock {
        session_id: row.get(0)?,
        original_hash: row.get(1)?,
        replacement: row.get(2)?,
        rules_version: row.get(3)?,
        ts: Timestamp(row.get(4)?),
    })
}

fn row_to_savings(row: &rusqlite::Row<'_>) -> rusqlite::Result<ShadowSavings> {
    Ok(ShadowSavings {
        kind: row.get(0)?,
        key: row.get(1)?,
        events: from_db(row.get(2)?),
        sessions: from_db(row.get(3)?),
        bytes_saved: from_db(row.get(4)?),
        est_tokens_saved: from_db(row.get(5)?),
    })
}

impl Store {
    /// Keeps an original tool result for `tokred_retrieve`; the content is stored as a BLOB.
    ///
    /// The id comes from the content hash, so the same original seen again (in this or another
    /// session) is the same row: its content is kept and only the later expiry wins, so a second
    /// session never shortens the first one's retention.
    pub fn put_stored_result(&self, result: &StoredResult) -> Result<(), StoreError> {
        self.conn
            .execute(
                &format!(
                    "INSERT INTO stored_results({STORED_COLUMNS}) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
                     ON CONFLICT(id) DO UPDATE SET \
                     expires_at = MAX(expires_at, excluded.expires_at)"
                ),
                params![
                    result.id,
                    result.session_id,
                    result.tool,
                    result.original_hash,
                    result.content.as_bytes(),
                    to_db(result.lines),
                    to_db(result.bytes),
                    result.created_at.0,
                    result.expires_at.0
                ],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }

    /// The stored result with this id, unless it expired (`expires_at <= now`).
    pub fn stored_result(
        &self,
        id: &str,
        now: Timestamp,
    ) -> Result<Option<StoredResult>, StoreError> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {STORED_COLUMNS} FROM stored_results WHERE id = ?1 AND expires_at > ?2"
                ),
                params![id, now.0],
                row_to_stored,
            )
            .optional()
            .map_err(StoreError::backend)
    }

    /// Deletes the stored results that expired (`expires_at <= now`); returns how many.
    pub fn prune_stored_results(&self, now: Timestamp) -> Result<usize, StoreError> {
        self.conn
            .execute(
                "DELETE FROM stored_results WHERE expires_at <= ?1",
                params![now.0],
            )
            .map_err(StoreError::backend)
    }

    /// Records the bytes sent for a replaced block. An existing row for the same conversation
    /// and hash is **kept**: the bytes first sent are the ones every later turn must repeat.
    /// Returns whether the row was inserted.
    pub fn insert_compressed_block(&self, block: &CompressedBlock) -> Result<bool, StoreError> {
        self.conn
            .execute(
                &format!(
                    "INSERT INTO compressed_blocks({BLOCK_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5) \
                     ON CONFLICT(session_id, original_hash) DO NOTHING"
                ),
                params![
                    block.session_id,
                    block.original_hash,
                    block.replacement,
                    block.rules_version,
                    block.ts.0
                ],
            )
            .map(|inserted| inserted > 0)
            .map_err(StoreError::backend)
    }

    /// The replaced blocks of one conversation (see [`CompressedBlock::session_id`]), oldest
    /// first.
    pub fn compressed_blocks(
        &self,
        conversation: &str,
    ) -> Result<Vec<CompressedBlock>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {BLOCK_COLUMNS} FROM compressed_blocks WHERE session_id = ?1 \
                 ORDER BY ts, original_hash"
            ))
            .map_err(StoreError::backend)?;
        stmt.query_map(params![conversation], row_to_block)
            .map_err(StoreError::backend)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::backend)
    }

    /// The replaced blocks first sent at or after `since`, in every conversation (to seed the
    /// proxy at startup), oldest first.
    pub fn recent_compressed_blocks(
        &self,
        since: Timestamp,
    ) -> Result<Vec<CompressedBlock>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {BLOCK_COLUMNS} FROM compressed_blocks WHERE ts >= ?1 \
                 ORDER BY ts, session_id, original_hash"
            ))
            .map_err(StoreError::backend)?;
        stmt.query_map(params![since.0], row_to_block)
            .map_err(StoreError::backend)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::backend)
    }

    /// Deletes the replaced blocks first sent before `before`; returns how many.
    pub fn prune_compressed_blocks(&self, before: Timestamp) -> Result<usize, StoreError> {
        self.conn
            .execute(
                "DELETE FROM compressed_blocks WHERE ts < ?1",
                params![before.0],
            )
            .map_err(StoreError::backend)
    }

    /// Inserts shadow-mode records in one transaction; returns how many.
    pub fn insert_shadow_records(&self, records: &[ShadowRecord]) -> Result<usize, StoreError> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(StoreError::backend)?;
        let mut inserted = 0;
        {
            let mut stmt = tx
                .prepare_cached(
                    "INSERT INTO shadow_savings(ts, session_id, kind, key, bytes_saved, \
                     est_tokens_saved, rules_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )
                .map_err(StoreError::backend)?;
            for record in records {
                inserted += stmt
                    .execute(params![
                        record.ts.0,
                        record.session_id,
                        record.kind,
                        record.key,
                        to_db(record.bytes_saved),
                        to_db(record.est_tokens_saved),
                        record.rules_version
                    ])
                    .map_err(StoreError::backend)?;
            }
        }
        tx.commit().map_err(StoreError::backend)?;
        Ok(inserted)
    }

    /// Shadow-mode savings per kind and key with `from <= ts < to`, ordered by kind and then by
    /// the largest estimated saving.
    pub fn shadow_savings(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<ShadowSavings>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT kind, key, COUNT(*), COUNT(DISTINCT session_id), SUM(bytes_saved), \
                 SUM(est_tokens_saved) FROM shadow_savings WHERE ts >= ?1 AND ts < ?2 \
                 GROUP BY kind, key ORDER BY kind, SUM(est_tokens_saved) DESC, key",
            )
            .map_err(StoreError::backend)?;
        stmt.query_map(params![from.0, to.0], row_to_savings)
            .map_err(StoreError::backend)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::backend)
    }

    /// Deletes the shadow-mode records older than `before`; returns how many.
    pub fn prune_shadow_savings(&self, before: Timestamp) -> Result<usize, StoreError> {
        self.conn
            .execute(
                "DELETE FROM shadow_savings WHERE ts < ?1",
                params![before.0],
            )
            .map_err(StoreError::backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_saturate_and_clamp() {
        assert_eq!(to_db(u64::MAX), i64::MAX);
        assert_eq!(from_db(-1), 0);
        assert_eq!(from_db(7), 7);
    }
}
