//! `[proxy] retention_days`: removing proxy data older than the retention.

use rusqlite::params;
use tokred_core::{StoreError, Timestamp};

use crate::Store;

const SECONDS_PER_DAY: i64 = 86_400;

/// How many rows [`Store::prune_proxy_data`] removed from each table.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// `proxy_requests`.
    pub requests: usize,
    /// `routing_decisions`.
    pub decisions: usize,
    /// `prompt_summaries`.
    pub prompts: usize,
    /// `suggestions`.
    pub suggestions: usize,
    /// `shadow_savings`.
    pub shadow: usize,
    /// `stored_results`.
    pub stored_results: usize,
    /// `compressed_blocks`.
    pub compressed_blocks: usize,
}

impl PruneReport {
    /// Rows removed in total.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.requests
            + self.decisions
            + self.prompts
            + self.suggestions
            + self.shadow
            + self.stored_results
            + self.compressed_blocks
    }
}

impl Store {
    /// Deletes the proxy's data older than `keep_days` days before `now`; `0` keeps everything.
    ///
    /// Nothing that belongs to a request still in flight is touched: those rows are newer than
    /// the cutoff (a day at least), and the original tool results and replacements of a
    /// conversation with a recent request are kept whatever their age.
    pub fn prune_proxy_data(
        &self,
        now: Timestamp,
        keep_days: u32,
    ) -> Result<PruneReport, StoreError> {
        if keep_days == 0 {
            return Ok(PruneReport::default());
        }
        let cutoff = now.0.saturating_sub(i64::from(keep_days) * SECONDS_PER_DAY);
        let by_age = |table: &str| {
            self.conn
                .execute(
                    &format!("DELETE FROM {table} WHERE ts < ?1"),
                    params![cutoff],
                )
                .map_err(StoreError::backend)
        };
        // Tool results and replacements belong to a conversation: one that still has a recent
        // request keeps them whatever their age, because a replaced tool result must be sent as
        // the same bytes on every turn (changing it would break the prompt cache) and the model
        // may still ask for the original.
        let unless_active = |table: &str, column: &str| {
            self.conn
                .execute(
                    &format!(
                        "DELETE FROM {table} WHERE {column} < ?1 AND session_id NOT IN \
                         (SELECT session_id FROM proxy_requests WHERE ts >= ?1)"
                    ),
                    params![cutoff],
                )
                .map_err(StoreError::backend)
        };
        Ok(PruneReport {
            requests: by_age("proxy_requests")?,
            decisions: by_age("routing_decisions")?,
            prompts: by_age("prompt_summaries")?,
            suggestions: by_age("suggestions")?,
            shadow: by_age("shadow_savings")?,
            stored_results: unless_active("stored_results", "created_at")?,
            compressed_blocks: unless_active("compressed_blocks", "ts")?,
        })
    }
}
