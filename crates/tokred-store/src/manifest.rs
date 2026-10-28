//! `installed_files` repository.

use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, params};
use tokred_core::{ManifestEntry, ManifestRepo, StoreError, Timestamp};

use crate::Store;

fn key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<ManifestEntry> {
    Ok(ManifestEntry {
        path: PathBuf::from(row.get::<_, String>(0)?),
        hash: row.get(1)?,
        version: row.get(2)?,
        installed_at: Timestamp(row.get(3)?),
    })
}

impl ManifestRepo for Store {
    fn get(&self, path: &Path) -> Result<Option<ManifestEntry>, StoreError> {
        self.conn
            .query_row(
                "SELECT path, hash, version, installed_at FROM installed_files WHERE path = ?1",
                params![key(path)],
                row_to_entry,
            )
            .optional()
            .map_err(StoreError::backend)
    }

    fn upsert(&self, entry: &ManifestEntry) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO installed_files(path, hash, version, installed_at) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(path) DO UPDATE SET hash = excluded.hash, \
                 version = excluded.version, installed_at = excluded.installed_at",
                params![
                    key(&entry.path),
                    entry.hash,
                    entry.version,
                    entry.installed_at.0
                ],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }

    fn remove(&self, path: &Path) -> Result<(), StoreError> {
        self.conn
            .execute(
                "DELETE FROM installed_files WHERE path = ?1",
                params![key(path)],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }

    fn list(&self) -> Result<Vec<ManifestEntry>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, hash, version, installed_at FROM installed_files ORDER BY path")
            .map_err(StoreError::backend)?;
        stmt.query_map([], row_to_entry)
            .map_err(StoreError::backend)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::backend)
    }
}
