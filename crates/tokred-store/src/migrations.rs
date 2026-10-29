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
    // v3: 1-hour cache writes (priced at 2x input instead of 1.25x). Clearing the ingestion
    // offsets makes the next `stats` re-read the transcripts; `insert_new` then backfills the
    // 1-hour split of rows that already exist without duplicating them.
    "
    ALTER TABLE usage ADD COLUMN cache_write_1h INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE pricing ADD COLUMN cache_write_1h REAL;
    DELETE FROM ingest_state;
    ",
    // v4: the local proxy (spec section 7.1). Fields a workstream needs later go in the JSON
    // `detail`/`notes` columns instead of new migrations.
    "
    CREATE TABLE proxy_requests(
        id TEXT PRIMARY KEY, ts INTEGER NOT NULL, session_id TEXT NOT NULL, agent_id TEXT,
        agent_type TEXT, prompt_id TEXT, origin TEXT NOT NULL, model_requested TEXT NOT NULL,
        model_effective TEXT NOT NULL, input INTEGER NOT NULL, output INTEGER NOT NULL,
        cache_write INTEGER NOT NULL, cache_write_1h INTEGER NOT NULL, cache_read INTEGER NOT NULL,
        thinking INTEGER NOT NULL, message_key TEXT, bytes_in INTEGER NOT NULL,
        bytes_out INTEGER NOT NULL, upstream_ttfb_ms INTEGER NOT NULL, total_ms INTEGER NOT NULL,
        proxy_overhead_us INTEGER NOT NULL, status INTEGER NOT NULL,
        notes TEXT NOT NULL DEFAULT '{}', miss_cause TEXT, tools_hash TEXT NOT NULL,
        system_hash TEXT NOT NULL
    );
    CREATE INDEX proxy_requests_ts ON proxy_requests(ts);
    CREATE INDEX proxy_requests_session ON proxy_requests(session_id, ts);
    CREATE TABLE routing_decisions(
        id TEXT PRIMARY KEY, ts INTEGER NOT NULL, session_id TEXT NOT NULL, agent_id TEXT,
        scope TEXT NOT NULL, task_class TEXT NOT NULL, confidence REAL NOT NULL,
        layer TEXT NOT NULL, reason TEXT NOT NULL, model_from TEXT NOT NULL,
        model_to TEXT NOT NULL, mode TEXT NOT NULL, est_cost_from REAL NOT NULL,
        est_cost_to REAL NOT NULL, rules_version TEXT NOT NULL, outcome TEXT NOT NULL,
        detail TEXT NOT NULL DEFAULT '{}'
    );
    CREATE INDEX routing_decisions_ts ON routing_decisions(ts);
    CREATE INDEX routing_decisions_agent ON routing_decisions(session_id, agent_id);
    CREATE TABLE prompt_summaries(
        prompt_id TEXT PRIMARY KEY, session_id TEXT NOT NULL, ts INTEGER NOT NULL,
        model TEXT NOT NULL, turns INTEGER NOT NULL, tools TEXT NOT NULL,
        output_tokens INTEGER NOT NULL, thinking_tokens INTEGER NOT NULL, cost_usd REAL NOT NULL,
        edited INTEGER NOT NULL, subagents INTEGER NOT NULL, simple INTEGER NOT NULL,
        est_saving_usd REAL NOT NULL, detail TEXT NOT NULL DEFAULT '{}'
    );
    CREATE INDEX prompt_summaries_session ON prompt_summaries(session_id, ts);
    CREATE TABLE suggestions(
        id TEXT PRIMARY KEY, ts INTEGER NOT NULL, session_id TEXT NOT NULL, kind TEXT NOT NULL,
        message TEXT NOT NULL, shown INTEGER NOT NULL DEFAULT 0, detail TEXT NOT NULL DEFAULT '{}'
    );
    CREATE INDEX suggestions_session ON suggestions(session_id, ts);
    CREATE TABLE shadow_savings(
        ts INTEGER NOT NULL, session_id TEXT NOT NULL, kind TEXT NOT NULL, key TEXT NOT NULL,
        bytes_saved INTEGER NOT NULL, est_tokens_saved INTEGER NOT NULL,
        rules_version TEXT NOT NULL
    );
    CREATE INDEX shadow_savings_ts ON shadow_savings(ts);
    CREATE TABLE stored_results(
        id TEXT PRIMARY KEY, session_id TEXT NOT NULL, tool TEXT NOT NULL,
        original_hash TEXT NOT NULL, content BLOB NOT NULL, lines INTEGER NOT NULL,
        bytes INTEGER NOT NULL, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL
    );
    CREATE INDEX stored_results_expires ON stored_results(expires_at);
    CREATE TABLE compressed_blocks(
        session_id TEXT NOT NULL, original_hash TEXT NOT NULL, replacement TEXT NOT NULL,
        rules_version TEXT NOT NULL, ts INTEGER NOT NULL, PRIMARY KEY(session_id, original_hash)
    );
    ",
    // v5: what the dashboard shows of a turn besides its request row. `stop_reason` and the
    // tools called are not message content and are kept for every turn; `sent_json` and
    // `response_json` hold content and stay NULL unless `[proxy] log_bodies` is on (and are
    // emptied again when the turn leaves the retention window).
    "
    CREATE TABLE captured_turns(
        request_id TEXT PRIMARY KEY, session_id TEXT NOT NULL, agent_id TEXT, ts INTEGER NOT NULL,
        stop_reason TEXT, tools_json TEXT NOT NULL DEFAULT '[]', sent_json TEXT,
        response_json TEXT
    );
    CREATE INDEX captured_turns_session ON captured_turns(session_id, ts);
    CREATE INDEX captured_turns_ts ON captured_turns(ts);
",
    // v6: richer per-request records. Existing rows stay valid (no stop reason, no tools, no
    // cache lifetime, unknown routing mode).
    "
    ALTER TABLE proxy_requests ADD COLUMN stop_reason TEXT;
    ALTER TABLE proxy_requests ADD COLUMN tools TEXT NOT NULL DEFAULT '[]';
    ALTER TABLE proxy_requests ADD COLUMN cache_ttl TEXT;
    ALTER TABLE proxy_requests ADD COLUMN routing_mode TEXT;
    ",
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
