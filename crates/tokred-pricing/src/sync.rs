//! Price use cases: sync, load and the "once a day" rule. Pure: every port is a parameter.

use tokred_core::error::StoreError;
use tokred_core::meta::MetaRepo;
use tokred_core::ports::Clock;
use tokred_core::pricing::{PriceRepo, PriceSource, PriceTable};
use tokred_core::types::Timestamp;

use crate::litellm::fallback_entries;

/// Meta key: when the table was last refreshed from the network (Unix seconds).
pub const PRICE_SYNCED_AT: &str = "price_synced_at";
/// Meta key: when a sync was last attempted, successful or not (Unix seconds). Used by
/// [`sync_due`] so an offline machine does not retry on every command.
pub const PRICE_SYNC_ATTEMPT_AT: &str = "price_sync_attempt_at";

/// Minimum time between automatic syncs.
const SYNC_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// Result of [`sync_prices`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// The table was replaced with fresh prices.
    Synced {
        /// Number of models now in the table.
        models: usize,
    },
    /// The sync failed; the previous table is kept.
    Failed {
        /// Why it failed.
        reason: String,
        /// Whether the table was empty and has been seeded with the embedded fallback.
        seeded_fallback: bool,
    },
}

/// Fetches prices and replaces the table.
///
/// On success the rows are stamped with the current time and `price_synced_at` is recorded. On
/// any failure (network, empty answer, store error) the existing table is left alone, an empty
/// table is seeded with the embedded fallback, and the reason is reported. A failed attempt is
/// still recorded (`price_sync_attempt_at`) so [`sync_due`] does not hammer the network.
pub fn sync_prices(
    source: &dyn PriceSource,
    repo: &dyn PriceRepo,
    meta: &dyn MetaRepo,
    clock: &dyn Clock,
) -> SyncOutcome {
    let now = clock.now();
    // Bookkeeping only: if these writes fail we simply sync again on the next due check.
    let _ = meta.meta_set(PRICE_SYNC_ATTEMPT_AT, &now.0.to_string());
    match fetch_and_store(source, repo, now) {
        Ok(models) => {
            let _ = meta.meta_set(PRICE_SYNCED_AT, &now.0.to_string());
            SyncOutcome::Synced { models }
        }
        Err(reason) => SyncOutcome::Failed {
            reason,
            seeded_fallback: seed_if_empty(repo, now),
        },
    }
}

fn fetch_and_store(
    source: &dyn PriceSource,
    repo: &dyn PriceRepo,
    now: Timestamp,
) -> Result<usize, String> {
    let mut entries = source.fetch().map_err(|err| err.to_string())?;
    if entries.is_empty() {
        return Err("the price source returned no models".to_owned());
    }
    for entry in &mut entries {
        entry.updated_at = now;
    }
    repo.replace_all(&entries).map_err(|err| err.to_string())?;
    Ok(entries.len())
}

/// Seeds the embedded fallback when the table is empty; returns whether it did.
fn seed_if_empty(repo: &dyn PriceRepo, now: Timestamp) -> bool {
    match repo.load() {
        Ok(rows) if rows.is_empty() => repo.replace_all(&fallback_entries(now)).is_ok(),
        _ => false,
    }
}

/// The price table to use for costs.
///
/// Reads the table from `repo`; when it is empty the embedded fallback is stored first. If the
/// repository cannot be read, the fallback is used from memory, so costs can always be computed.
#[must_use]
pub fn load_table(repo: &dyn PriceRepo, clock: &dyn Clock) -> PriceTable {
    match repo.load() {
        Ok(rows) if !rows.is_empty() => PriceTable::new(rows),
        Ok(_) => {
            let entries = fallback_entries(clock.now());
            // Best effort: the in-memory table is valid even if persisting it fails.
            let _ = repo.replace_all(&entries);
            PriceTable::new(entries)
        }
        Err(_) => PriceTable::new(fallback_entries(clock.now())),
    }
}

/// Whether the daily automatic sync should run now: `auto_sync` is on and the last attempt is at
/// least 24 hours old (or missing, unreadable or in the future). A broken meta store means "no",
/// so a failing database never causes a request on every command.
#[must_use]
pub fn sync_due(meta: &dyn MetaRepo, clock: &dyn Clock, auto_sync: bool) -> bool {
    if !auto_sync {
        return false;
    }
    let (Ok(synced), Ok(attempted)) = (
        read_timestamp(meta, PRICE_SYNCED_AT),
        read_timestamp(meta, PRICE_SYNC_ATTEMPT_AT),
    ) else {
        return false;
    };
    let last = synced.max(attempted);
    last.is_none_or(|last| !(0..SYNC_INTERVAL_SECS).contains(&(clock.now().0 - last)))
}

/// A stored Unix timestamp; unparsable values count as absent.
fn read_timestamp(meta: &dyn MetaRepo, key: &str) -> Result<Option<i64>, StoreError> {
    Ok(meta
        .meta_get(key)?
        .and_then(|value| value.trim().parse().ok()))
}

#[cfg(test)]
mod tests;
