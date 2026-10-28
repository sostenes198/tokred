//! Versioned schema migrations (`PRAGMA user_version`).

use rusqlite::Connection;

use crate::OpenError;

/// Migration `N` is `MIGRATIONS[N - 1]`; the schema version is the number applied.
const MIGRATIONS: &[&str] = &[
    // v1: full schema from spec section 8.
    "
    CREATE TABLE sessions(
        id TEXT PRIMARY KEY, project TEXT, started_at INTEGER, last_seen INTEGER,
        last_reset_at INTEGER
    );
    CREATE TABLE classifications(
        session_id TEXT, ts INTEGER, class TEXT, prompt_len INTEGER
    );
    CREATE TABLE file_reads(
        session_id TEXT, path TEXT, hash TEXT, offset INTEGER, lim INTEGER, bytes INTEGER,
        ts INTEGER,
        PRIMARY KEY(session_id, path, hash, offset, lim)
    );
    CREATE TABLE hook_events(
        ts INTEGER, session_id TEXT, hook TEXT, decision TEXT, reason TEXT, bytes_saved INTEGER,
        duration_us INTEGER, outcome TEXT
    );
    CREATE TABLE cache_entries(
        key TEXT PRIMARY KEY, kind TEXT, value BLOB, tier INTEGER, hits INTEGER,
        created_at INTEGER, last_hit INTEGER, expires_at INTEGER
    );
    CREATE TABLE installed_files(
        path TEXT PRIMARY KEY, hash TEXT, version TEXT, installed_at INTEGER
    );
    CREATE TABLE pricing(
        model TEXT PRIMARY KEY, input REAL, output REAL, cache_write REAL, cache_read REAL,
        source TEXT, updated_at INTEGER
    );
    CREATE TABLE usage(
        message_key TEXT PRIMARY KEY, session_id TEXT, project TEXT, ts INTEGER, model TEXT,
        input INTEGER, output INTEGER, cache_write INTEGER, cache_read INTEGER,
        is_subagent INTEGER
    );
    CREATE TABLE ingest_state(file TEXT PRIMARY KEY, offset INTEGER, mtime INTEGER);
    ",
    // v2: key/value state that has no table of its own.
    "CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);",
];

/// Highest schema version this build understands.
pub(crate) fn latest() -> u32 {
    u32::try_from(MIGRATIONS.len()).unwrap_or(u32::MAX)
}

/// Applies every pending migration, each in its own transaction.
pub(crate) fn apply(conn: &Connection) -> Result<(), OpenError> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current > latest() {
        return Err(OpenError::TooNew {
            found: current,
            supported: latest(),
        });
    }
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let version = index + 1;
        conn.execute_batch(&format!(
            "BEGIN;\n{sql}\nPRAGMA user_version = {version};\nCOMMIT;"
        ))?;
    }
    Ok(())
}
