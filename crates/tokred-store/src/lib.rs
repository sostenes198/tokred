//! SQLite adapter for tokred.
//!
//! Opens the database (WAL mode, 2 s busy timeout), applies versioned migrations tracked in
//! `PRAGMA user_version`, and implements the repository ports defined in `tokred-core`.

mod cache;
mod hook_event;
mod manifest;
mod meta;
mod migrations;
mod pricing;
mod session;
mod usage;

use std::path::Path;

use rusqlite::Connection;

/// Failure while opening or migrating the database.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// SQLite reported an error.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The database was created by a newer tokred.
    #[error("database schema version {found} is newer than supported version {supported}")]
    TooNew {
        /// Version found in the file.
        found: u32,
        /// Highest version this build understands.
        supported: u32,
    },
}

const TABLE_NAMES_SQL: &str =
    "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name";

/// Handle to the tokred database.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens (creating if needed) the database at `path` and migrates it to the latest schema.
    pub fn open(path: &Path) -> Result<Self, OpenError> {
        Self::from_connection(Connection::open(path)?)
    }

    /// Opens an in-memory database (tests).
    pub fn open_in_memory() -> Result<Self, OpenError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, OpenError> {
        conn.busy_timeout(std::time::Duration::from_millis(2000))?;
        // `journal_mode` returns a row, so it must be read rather than executed.
        let _mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        migrations::apply(&conn)?;
        Ok(Self { conn })
    }

    /// The schema version this build migrates databases to.
    #[must_use]
    pub fn latest_schema_version() -> u32 {
        migrations::latest()
    }

    /// Current schema version (`PRAGMA user_version`).
    pub fn schema_version(&self) -> Result<u32, rusqlite::Error> {
        self.conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
    }

    /// Names of the user tables, sorted.
    pub fn table_names(&self) -> Result<Vec<String>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(TABLE_NAMES_SQL)?;
        stmt.query_map([], |row| row.get(0))?.collect()
    }
}
