//! The figures behind the savings verdicts, and the weekly schedule of the savings hint.

use rusqlite::params;
use tokred_core::StoreError;
use tokred_core::Timestamp;
use tokred_core::savings::{SavingsFigures, SavingsRepo};

use crate::Store;

/// The `meta` key holding when the savings hint is next due (unix seconds).
const NEXT_KEY: &str = "savings_hint_next";
/// Seconds in a day, to turn the period into days.
const DAY_SECS: f64 = 86_400.0;

impl SavingsRepo for Store {
    fn savings_figures(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<SavingsFigures, StoreError> {
        let (routing_decisions, routing_changed, routing_saving_usd): (i64, i64, f64) = self
            .conn
            .query_row(
                "SELECT COUNT(*), \
                        COALESCE(SUM(outcome IN ('applied', 'observed')), 0), \
                        COALESCE(SUM(CASE WHEN outcome IN ('applied', 'observed') \
                                          THEN est_cost_from - est_cost_to ELSE 0 END), 0.0) \
                 FROM routing_decisions WHERE ts >= ?1 AND ts < ?2",
                params![from.0, to.0],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(StoreError::backend)?;
        let compression_tokens: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(est_tokens_saved), 0) FROM shadow_savings \
                 WHERE kind = 'compression' AND ts >= ?1 AND ts < ?2",
                params![from.0, to.0],
                |row| row.get(0),
            )
            .map_err(StoreError::backend)?;
        let input_tokens: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(SUM(input + cache_read + cache_write), 0) FROM proxy_requests \
                 WHERE ts >= ?1 AND ts < ?2",
                params![from.0, to.0],
                |row| row.get(0),
            )
            .map_err(StoreError::backend)?;
        Ok(SavingsFigures {
            #[allow(clippy::cast_precision_loss)]
            days: (to.0 - from.0) as f64 / DAY_SECS,
            routing_decisions: u64::try_from(routing_decisions).unwrap_or(0),
            routing_changed: u64::try_from(routing_changed).unwrap_or(0),
            routing_saving_usd: routing_saving_usd.max(0.0),
            compression_tokens: u64::try_from(compression_tokens).unwrap_or(0),
            input_tokens: u64::try_from(input_tokens).unwrap_or(0),
        })
    }

    fn claim_savings_hint(&self, now: Timestamp, every_secs: i64) -> Result<bool, StoreError> {
        // One statement: it inserts or moves the date only when it is due, so of two callers at the
        // same time just one changes a row.
        let changed = self
            .conn
            .execute(
                "INSERT INTO meta(key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value \
                 WHERE CAST(meta.value AS INTEGER) <= ?3",
                params![NEXT_KEY, (now.0 + every_secs).to_string(), now.0],
            )
            .map_err(StoreError::backend)?;
        Ok(changed == 1)
    }
}
