//! Repository for `hook_events`.

use rusqlite::params;
use tokred_core::hook_event::{HookEventRecord, HookEventRepo, Outcome};
use tokred_core::{StoreError, Timestamp};

use crate::Store;
use crate::session::{from_db, to_db};

impl HookEventRepo for Store {
    fn insert_event(&self, event: &HookEventRecord) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO hook_events(ts, session_id, hook, decision, reason, bytes_saved, \
                 duration_us, outcome) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    event.ts.0,
                    event.session_id,
                    event.hook,
                    event.decision,
                    event.reason,
                    to_db(event.bytes_saved),
                    to_db(event.duration_us),
                    event.outcome.as_str()
                ],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }

    fn events_between(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<HookEventRecord>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT ts, session_id, hook, decision, reason, bytes_saved, duration_us, outcome \
                 FROM hook_events WHERE ts >= ?1 AND ts < ?2 ORDER BY ts, rowid",
            )
            .map_err(StoreError::backend)?;
        stmt.query_map(params![from.0, to.0], |row| {
            let outcome: String = row.get(7)?;
            Ok(HookEventRecord {
                ts: Timestamp(row.get(0)?),
                session_id: row.get(1)?,
                hook: row.get(2)?,
                decision: row.get(3)?,
                reason: row.get(4)?,
                bytes_saved: from_db(row.get(5)?),
                duration_us: from_db(row.get(6)?),
                outcome: Outcome::parse(&outcome).unwrap_or(Outcome::Error),
            })
        })
        .map_err(StoreError::backend)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::backend)
    }
}
