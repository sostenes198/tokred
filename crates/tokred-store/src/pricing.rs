//! `pricing` table repository.

use rusqlite::params;
use tokred_core::StoreError;
use tokred_core::Timestamp;
use tokred_core::pricing::{ModelPrice, PriceEntry, PriceRepo};

use crate::Store;

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<PriceEntry> {
    Ok(PriceEntry {
        model: row.get(0)?,
        price: ModelPrice {
            input: row.get(1)?,
            output: row.get(2)?,
            cache_write: row.get(3)?,
            cache_read: row.get(4)?,
        },
        source: row.get(5)?,
        updated_at: Timestamp(row.get(6)?),
    })
}

impl PriceRepo for Store {
    fn replace_all(&self, entries: &[PriceEntry]) -> Result<(), StoreError> {
        // One transaction: a failure half way leaves the previous table untouched.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(StoreError::backend)?;
        tx.execute("DELETE FROM pricing", [])
            .map_err(StoreError::backend)?;
        {
            let mut insert = tx
                .prepare(
                    "INSERT INTO pricing(model, input, output, cache_write, cache_read, source, \
                     updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                     ON CONFLICT(model) DO UPDATE SET input = excluded.input, \
                     output = excluded.output, cache_write = excluded.cache_write, \
                     cache_read = excluded.cache_read, source = excluded.source, \
                     updated_at = excluded.updated_at",
                )
                .map_err(StoreError::backend)?;
            for entry in entries {
                insert
                    .execute(params![
                        entry.model,
                        entry.price.input,
                        entry.price.output,
                        entry.price.cache_write,
                        entry.price.cache_read,
                        entry.source,
                        entry.updated_at.0
                    ])
                    .map_err(StoreError::backend)?;
            }
        }
        tx.commit().map_err(StoreError::backend)
    }

    fn load(&self) -> Result<Vec<PriceEntry>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT model, input, output, cache_write, cache_read, source, updated_at \
                 FROM pricing ORDER BY model",
            )
            .map_err(StoreError::backend)?;
        stmt.query_map([], row_to_entry)
            .map_err(StoreError::backend)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::backend)
    }
}
