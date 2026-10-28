//! `usage` and `ingest_state` repositories (used by transcript ingestion and `stats`).

use rusqlite::{OptionalExtension, params};
use tokred_core::pricing::Usage;
use tokred_core::usage::{IngestRepo, IngestState, UsageRecord, UsageRepo};
use tokred_core::{StoreError, Timestamp};

use crate::Store;

const SELECT_USAGE: &str = "SELECT message_key, session_id, project, ts, model, input, output, \
                            cache_write, cache_read, is_subagent FROM usage";

/// SQLite integers are signed: counters beyond `i64::MAX` saturate (unreachable in practice).
fn to_db(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Counters are never negative; a corrupt negative value reads as zero.
fn from_db(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageRecord> {
    Ok(UsageRecord {
        message_key: row.get(0)?,
        session_id: row.get(1)?,
        project: row.get(2)?,
        ts: Timestamp(row.get(3)?),
        model: row.get(4)?,
        usage: Usage {
            input: from_db(row.get(5)?),
            output: from_db(row.get(6)?),
            cache_write: from_db(row.get(7)?),
            cache_read: from_db(row.get(8)?),
        },
        is_subagent: row.get::<_, i64>(9)? != 0,
    })
}

impl UsageRepo for Store {
    fn insert_new(&self, records: &[UsageRecord]) -> Result<usize, StoreError> {
        // One transaction for the whole batch: thousands of rows insert in milliseconds.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(StoreError::backend)?;
        let mut inserted = 0;
        {
            let mut stmt = tx
                .prepare_cached(
                    "INSERT OR IGNORE INTO usage(message_key, session_id, project, ts, model, \
                     input, output, cache_write, cache_read, is_subagent) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )
                .map_err(StoreError::backend)?;
            for record in records {
                inserted += stmt
                    .execute(params![
                        record.message_key,
                        record.session_id,
                        record.project,
                        record.ts.0,
                        record.model,
                        to_db(record.usage.input),
                        to_db(record.usage.output),
                        to_db(record.usage.cache_write),
                        to_db(record.usage.cache_read),
                        i64::from(record.is_subagent),
                    ])
                    .map_err(StoreError::backend)?;
            }
        }
        tx.commit().map_err(StoreError::backend)?;
        Ok(inserted)
    }

    fn usage_between(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<UsageRecord>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{SELECT_USAGE} WHERE ts >= ?1 AND ts < ?2 ORDER BY ts, message_key"
            ))
            .map_err(StoreError::backend)?;
        stmt.query_map(params![from.0, to.0], row_to_record)
            .map_err(StoreError::backend)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::backend)
    }

    fn earliest_usage(&self) -> Result<Option<Timestamp>, StoreError> {
        let min: Option<i64> = self
            .conn
            .query_row("SELECT MIN(ts) FROM usage", [], |row| row.get(0))
            .map_err(StoreError::backend)?;
        Ok(min.map(Timestamp))
    }
}

impl IngestRepo for Store {
    fn ingest_state(&self, file: &str) -> Result<Option<IngestState>, StoreError> {
        self.conn
            .query_row(
                "SELECT file, offset, mtime FROM ingest_state WHERE file = ?1",
                params![file],
                |row| {
                    Ok(IngestState {
                        file: row.get(0)?,
                        offset: from_db(row.get(1)?),
                        mtime: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::backend)
    }

    fn save_ingest_state(&self, state: &IngestState) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO ingest_state(file, offset, mtime) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(file) DO UPDATE SET offset = excluded.offset, \
                 mtime = excluded.mtime",
                params![state.file, to_db(state.offset), state.mtime],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }
}
