//! Repositories for `sessions`, `file_reads` and `classifications`.

use rusqlite::{OptionalExtension, params};
use tokred_core::session::{
    Classification, ClassificationRepo, FileRead, FileReadRepo, Session, SessionRepo, TaskClass,
};
use tokred_core::{StoreError, Timestamp};

use crate::Store;

/// `file_reads` stores an absent `offset`/`limit` as `-1`: `NULL`s in a primary key are
/// distinct in SQLite, so a `NULL` could never be found again.
const ABSENT: i64 = -1;

pub(crate) fn to_db(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(crate) fn from_db(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn range_to_db(value: Option<u64>) -> i64 {
    value.map_or(ABSENT, to_db)
}

impl SessionRepo for Store {
    fn touch_session(
        &self,
        id: &str,
        project: Option<&str>,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO sessions(id, project, started_at, last_seen, last_reset_at) \
                 VALUES (?1, ?2, ?3, ?3, NULL) \
                 ON CONFLICT(id) DO UPDATE SET last_seen = excluded.last_seen, \
                 project = COALESCE(excluded.project, sessions.project)",
                params![id, project, now.0],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }

    fn mark_reset(&self, id: &str, now: Timestamp) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO sessions(id, project, started_at, last_seen, last_reset_at) \
                 VALUES (?1, NULL, ?2, ?2, ?2) \
                 ON CONFLICT(id) DO UPDATE SET last_seen = excluded.last_seen, \
                 last_reset_at = excluded.last_reset_at",
                params![id, now.0],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }

    fn get_session(&self, id: &str) -> Result<Option<Session>, StoreError> {
        self.conn
            .query_row(
                "SELECT id, project, started_at, last_seen, last_reset_at \
                 FROM sessions WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Session {
                        id: row.get(0)?,
                        project: row.get(1)?,
                        started_at: Timestamp(row.get(2)?),
                        last_seen: Timestamp(row.get(3)?),
                        last_reset_at: row.get::<_, Option<i64>>(4)?.map(Timestamp),
                    })
                },
            )
            .optional()
            .map_err(StoreError::backend)
    }
}

impl FileReadRepo for Store {
    fn find_read(
        &self,
        scope: &str,
        path: &str,
        hash: &str,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> Result<Option<FileRead>, StoreError> {
        self.conn
            .query_row(
                "SELECT bytes, ts FROM file_reads \
                 WHERE session_id = ?1 AND path = ?2 AND hash = ?3 AND offset = ?4 AND lim = ?5",
                params![scope, path, hash, range_to_db(offset), range_to_db(limit)],
                |row| {
                    Ok(FileRead {
                        session_id: scope.to_owned(),
                        path: path.to_owned(),
                        hash: hash.to_owned(),
                        offset,
                        limit,
                        bytes: from_db(row.get(0)?),
                        ts: Timestamp(row.get(1)?),
                    })
                },
            )
            .optional()
            .map_err(StoreError::backend)
    }

    fn record_read(&self, read: &FileRead) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO file_reads(session_id, path, hash, offset, lim, bytes, ts) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT(session_id, path, hash, offset, lim) \
                 DO UPDATE SET bytes = excluded.bytes, ts = excluded.ts",
                params![
                    read.session_id,
                    read.path,
                    read.hash,
                    range_to_db(read.offset),
                    range_to_db(read.limit),
                    to_db(read.bytes),
                    read.ts.0
                ],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }
}

impl ClassificationRepo for Store {
    fn last_classification(&self, session_id: &str) -> Result<Option<Classification>, StoreError> {
        self.conn
            .query_row(
                "SELECT ts, class, prompt_len FROM classifications \
                 WHERE session_id = ?1 ORDER BY ts DESC, rowid DESC LIMIT 1",
                params![session_id],
                |row| {
                    let class: String = row.get(1)?;
                    Ok(Classification {
                        session_id: session_id.to_owned(),
                        ts: Timestamp(row.get(0)?),
                        class: TaskClass::parse(&class).unwrap_or(TaskClass::Other),
                        prompt_len: from_db(row.get(2)?),
                    })
                },
            )
            .optional()
            .map_err(StoreError::backend)
    }

    fn insert_classification(&self, classification: &Classification) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO classifications(session_id, ts, class, prompt_len) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    classification.session_id,
                    classification.ts.0,
                    classification.class.as_str(),
                    to_db(classification.prompt_len)
                ],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }
}
