//! `captured_turns` and the read queries behind the dashboard (`tokred_core::dashboard`). The
//! store holds the contents of a turn as the JSON text the proxy wrote; reading it is the
//! caller's job, so this crate needs no JSON library.

use rusqlite::params;
use tokred_core::dashboard::{
    ACTIVE_WINDOW_SECS, CapturedRow, CapturedTurn, SessionModelRow, StoredTurn,
};
use tokred_core::pricing::Usage;
use tokred_core::{StoreError, Timestamp};

use crate::Store;
use crate::proxy::{COLUMNS, row_to_record};

const SECONDS_PER_DAY: i64 = 86_400;

/// Per session and model sums of the newest sessions (`?1` limits to one session when set).
const SESSION_MODELS: &str = "WITH recent AS (\
    SELECT session_id, MIN(ts) AS first_ts, MAX(ts) AS last_ts FROM proxy_requests \
    WHERE session_id <> '' AND (?1 IS NULL OR session_id = ?1) \
    GROUP BY session_id ORDER BY last_ts DESC LIMIT ?2) \
    SELECT p.session_id, r.first_ts, r.last_ts, p.model_effective, COUNT(*), SUM(p.input), \
    SUM(p.output), SUM(p.cache_write), SUM(p.cache_write_1h), SUM(p.cache_read), \
    (SELECT COUNT(*) FROM captured_turns c WHERE c.session_id = p.session_id \
     AND c.sent_json IS NOT NULL) \
    FROM proxy_requests p JOIN recent r ON r.session_id = p.session_id \
    GROUP BY p.session_id, p.model_effective \
    ORDER BY r.last_ts DESC, p.session_id, MIN(p.ts)";

/// SQLite integers are signed; sums are never negative, a corrupt negative one reads as zero.
fn from_db(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn to_db(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn model_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionModelRow> {
    Ok(SessionModelRow {
        session_id: row.get(0)?,
        first_ts: Timestamp(row.get(1)?),
        last_ts: Timestamp(row.get(2)?),
        model: row.get(3)?,
        requests: from_db(row.get(4)?),
        usage: Usage {
            input: from_db(row.get(5)?),
            output: from_db(row.get(6)?),
            cache_write: from_db(row.get(7)?),
            cache_write_1h: from_db(row.get(8)?),
            cache_read: from_db(row.get(9)?),
        },
        content_turns: from_db(row.get(10)?),
    })
}

/// The columns the joined parts of a turn start at: right after [`COLUMNS`](crate::proxy::COLUMNS).
const JOINED: usize = crate::proxy::COLUMN_COUNT;

fn turn_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredTurn> {
    let record = row_to_record(row)?;
    let captured = match row.get::<_, Option<String>>(JOINED)? {
        Some(_) => Some(CapturedRow {
            stop_reason: row.get(JOINED + 1)?,
            tools_json: row.get(JOINED + 2)?,
            sent_json: row.get(JOINED + 3)?,
            response_json: row.get(JOINED + 4)?,
        }),
        None => None,
    };
    let routing = match row.get::<_, Option<String>>(JOINED + 5)? {
        Some(outcome) => Some((
            outcome,
            row.get::<_, Option<String>>(JOINED + 6)?
                .unwrap_or_default(),
        )),
        None => None,
    };
    Ok(StoredTurn {
        record,
        captured,
        routing,
    })
}

impl Store {
    /// Stores what the dashboard shows of a turn. A turn stored twice keeps the last version.
    pub fn insert_captured_turn(&self, turn: &CapturedTurn) -> Result<(), StoreError> {
        self.conn
            .prepare_cached(
                "INSERT OR REPLACE INTO captured_turns(request_id, session_id, agent_id, ts, \
                 stop_reason, tools_json, sent_json, response_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .and_then(|mut stmt| {
                stmt.execute(params![
                    turn.request_id,
                    turn.session_id,
                    turn.agent_id,
                    turn.ts.0,
                    turn.stop_reason,
                    turn.tools_json,
                    turn.sent_json,
                    turn.response_json,
                ])
            })
            .map(|_| ())
            .map_err(StoreError::backend)
    }

    /// What the `limit` sessions with the newest requests did, one row per session and model
    /// (the model that answered, first used first), newest session first. `only` restricts the
    /// result to one session. Requests without a session id are left out.
    pub fn session_model_rows(
        &self,
        only: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionModelRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare_cached(SESSION_MODELS)
            .map_err(StoreError::backend)?;
        stmt.query_map(params![only, to_db(limit)], model_row)
            .map_err(StoreError::backend)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::backend)
    }

    /// The newest `limit` turns of a session, oldest first, and whether older ones exist.
    /// Turns of the same second keep the order they were written in.
    pub fn session_turns(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<(Vec<StoredTurn>, bool), StoreError> {
        let sql = format!(
            "SELECT {COLUMNS}, cap_id, cap_stop, cap_tools, cap_sent, cap_response, dec_outcome, \
             dec_reason FROM proxy_requests \
             LEFT JOIN (SELECT request_id AS cap_id, stop_reason AS cap_stop, \
                 tools_json AS cap_tools, sent_json AS cap_sent, response_json AS cap_response \
                 FROM captured_turns) ON cap_id = id \
             LEFT JOIN (SELECT id AS dec_id, outcome AS dec_outcome, reason AS dec_reason \
                 FROM routing_decisions) ON dec_id = json_extract(notes, '$.decision_id') \
             WHERE session_id = ?1 ORDER BY ts DESC, proxy_requests.rowid DESC LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql).map_err(StoreError::backend)?;
        let mut turns = stmt
            .query_map(
                params![session_id, to_db(limit.saturating_add(1))],
                turn_row,
            )
            .map_err(StoreError::backend)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::backend)?;
        let truncated = turns.len() > limit;
        turns.truncate(limit);
        turns.reverse();
        Ok((turns, truncated))
    }

    /// Applies the retention rules of captured turns and returns how many rows changed:
    /// rows older than `keep_days` days are deleted (`0` keeps them all); the contents (not the stop reason and
    /// tools) of turns of sessions that are neither active nor among the `keep_sessions` most
    /// recent idle or ended ones are emptied.
    pub fn prune_captured_turns(
        &self,
        now: Timestamp,
        keep_days: u32,
        keep_sessions: usize,
    ) -> Result<usize, StoreError> {
        let cutoff = now.0.saturating_sub(i64::from(keep_days) * SECONDS_PER_DAY);
        let active_since = now.0.saturating_sub(ACTIVE_WINDOW_SECS);
        // 0 keeps every turn (only the contents are trimmed), like the other proxy tables.
        let deleted = if keep_days == 0 {
            0
        } else {
            self.conn
                .execute("DELETE FROM captured_turns WHERE ts < ?1", params![cutoff])
                .map_err(StoreError::backend)?
        };
        let emptied = self
            .conn
            .execute(
                "UPDATE captured_turns SET sent_json = NULL, response_json = NULL \
                 WHERE (sent_json IS NOT NULL OR response_json IS NOT NULL) \
                 AND session_id NOT IN (SELECT session_id FROM captured_turns \
                     GROUP BY session_id HAVING MAX(ts) >= ?1) \
                 AND session_id NOT IN (SELECT session_id FROM captured_turns \
                     GROUP BY session_id HAVING MAX(ts) < ?1 ORDER BY MAX(ts) DESC LIMIT ?2)",
                params![active_since, to_db(keep_sessions)],
            )
            .map_err(StoreError::backend)?;
        Ok(deleted + emptied)
    }
}
