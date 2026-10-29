//! `proxy_requests` repository (proxy workstream): one row per `/v1/messages` request seen by the
//! local proxy. The hook notes live in the JSON `notes` column, written and read with SQLite's
//! built-in JSON functions so this crate needs no JSON library.

use rusqlite::params;
use tokred_core::pricing::Usage;
use tokred_core::proxy::{HookNotes, ProxyRequestRecord};
use tokred_core::{StoreError, Timestamp};

use crate::Store;

const SECONDS_PER_DAY: i64 = 86_400;

const INSERT: &str = "INSERT OR IGNORE INTO proxy_requests(id, ts, session_id, agent_id, \
    agent_type, prompt_id, origin, model_requested, model_effective, input, output, cache_write, \
    cache_write_1h, cache_read, thinking, message_key, bytes_in, bytes_out, upstream_ttfb_ms, \
    total_ms, proxy_overhead_us, status, notes, miss_cause, tools_hash, system_hash) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, \
    ?20, ?21, ?22, json_object('decision_id', ?23, 'model_rewritten', ?24, \
    'blocks_compressed', ?25), ?26, ?27, ?28)";

/// The columns [`row_to_record`] reads, in order (unqualified: queries that join other tables
/// rename the columns of those).
pub(crate) const COLUMNS: &str = "id, ts, session_id, agent_id, agent_type, prompt_id, origin, \
    model_requested, model_effective, input, output, cache_write, cache_write_1h, cache_read, \
    thinking, message_key, bytes_in, bytes_out, upstream_ttfb_ms, total_ms, proxy_overhead_us, \
    status, json_extract(notes, '$.decision_id'), json_extract(notes, '$.model_rewritten'), \
    COALESCE(json_extract(notes, '$.blocks_compressed'), 0), miss_cause, tools_hash, system_hash";

/// SQLite integers are signed: counters beyond `i64::MAX` saturate (unreachable in practice).
fn to_db(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Counters are never negative; a corrupt negative value reads as zero.
fn from_db(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

pub(crate) fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProxyRequestRecord> {
    Ok(ProxyRequestRecord {
        id: row.get(0)?,
        ts: Timestamp(row.get(1)?),
        session_id: row.get(2)?,
        agent_id: row.get(3)?,
        agent_type: row.get(4)?,
        prompt_id: row.get(5)?,
        origin: row.get(6)?,
        model_requested: row.get(7)?,
        model_effective: row.get(8)?,
        usage: Usage {
            input: from_db(row.get(9)?),
            output: from_db(row.get(10)?),
            cache_write: from_db(row.get(11)?),
            cache_write_1h: from_db(row.get(12)?),
            cache_read: from_db(row.get(13)?),
        },
        thinking_tokens: from_db(row.get(14)?),
        message_key: row.get(15)?,
        bytes_in: from_db(row.get(16)?),
        bytes_out: from_db(row.get(17)?),
        upstream_ttfb_ms: from_db(row.get(18)?),
        total_ms: from_db(row.get(19)?),
        proxy_overhead_us: from_db(row.get(20)?),
        status: u16::try_from(row.get::<_, i64>(21)?).unwrap_or(0),
        notes: HookNotes {
            decision_id: row.get(22)?,
            model_rewritten: row.get(23)?,
            blocks_compressed: u32::try_from(row.get::<_, i64>(24)?).unwrap_or(0),
        },
        miss_cause: row.get(25)?,
        tools_hash: row.get(26)?,
        system_hash: row.get(27)?,
    })
}

impl Store {
    /// Stores one proxy request. A row with the same id is left as it is (the insert is
    /// idempotent). Returns whether a row was inserted.
    pub fn insert_proxy_request(&self, record: &ProxyRequestRecord) -> Result<bool, StoreError> {
        let inserted = self
            .conn
            .prepare_cached(INSERT)
            .and_then(|mut stmt| {
                stmt.execute(params![
                    record.id,
                    record.ts.0,
                    record.session_id,
                    record.agent_id,
                    record.agent_type,
                    record.prompt_id,
                    record.origin,
                    record.model_requested,
                    record.model_effective,
                    to_db(record.usage.input),
                    to_db(record.usage.output),
                    to_db(record.usage.cache_write),
                    to_db(record.usage.cache_write_1h),
                    to_db(record.usage.cache_read),
                    to_db(record.thinking_tokens),
                    record.message_key,
                    to_db(record.bytes_in),
                    to_db(record.bytes_out),
                    to_db(record.upstream_ttfb_ms),
                    to_db(record.total_ms),
                    to_db(record.proxy_overhead_us),
                    i64::from(record.status),
                    record.notes.decision_id,
                    record.notes.model_rewritten,
                    i64::from(record.notes.blocks_compressed),
                    record.miss_cause,
                    record.tools_hash,
                    record.system_hash,
                ])
            })
            .map_err(StoreError::backend)?;
        Ok(inserted > 0)
    }

    /// Proxy requests with `from <= ts < to`, oldest first.
    pub fn proxy_requests_between(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<ProxyRequestRecord>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM proxy_requests WHERE ts >= ?1 AND ts < ?2 ORDER BY ts, id"
            ))
            .map_err(StoreError::backend)?;
        stmt.query_map(params![from.0, to.0], row_to_record)
            .map_err(StoreError::backend)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::backend)
    }

    /// Deletes proxy requests older than `keep_days` days before `now`. Returns how many rows
    /// were removed.
    pub fn prune_proxy_requests(
        &self,
        now: Timestamp,
        keep_days: u32,
    ) -> Result<usize, StoreError> {
        let cutoff = now.0.saturating_sub(i64::from(keep_days) * SECONDS_PER_DAY);
        self.conn
            .execute("DELETE FROM proxy_requests WHERE ts < ?1", params![cutoff])
            .map_err(StoreError::backend)
    }
}
