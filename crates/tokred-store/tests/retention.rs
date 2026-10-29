//! `[proxy] retention_days` pruning against a real SQLite file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rusqlite::{Connection, params};
use tokred_core::Timestamp;
use tokred_store::{PruneReport, Store};

const DAY: i64 = 86_400;
const NOW: i64 = 200 * DAY;

struct Db {
    _dir: tempfile::TempDir,
    store: Store,
    conn: Connection,
}

fn open() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let store = Store::open(&path).unwrap();
    let conn = Connection::open(&path).unwrap();
    Db {
        _dir: dir,
        store,
        conn,
    }
}

impl Db {
    fn count(&self, table: &str) -> i64 {
        self.conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    /// One row of every pruned table, written at `ts` for conversation `session`.
    fn seed(&self, id: &str, session: &str, ts: i64) {
        self.conn
            .execute(
                "INSERT INTO proxy_requests(id, ts, session_id, origin, model_requested, \
                 model_effective, input, output, cache_write, cache_write_1h, cache_read, \
                 thinking, bytes_in, bytes_out, upstream_ttfb_ms, total_ms, proxy_overhead_us, \
                 status, tools_hash, system_hash) \
                 VALUES (?1, ?2, ?3, 'main', 'm', 'm', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 200, '', '')",
                params![id, ts, session],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO routing_decisions(id, ts, session_id, scope, task_class, confidence, \
                 layer, reason, model_from, model_to, mode, est_cost_from, est_cost_to, \
                 rules_version, outcome) \
                 VALUES (?1, ?2, ?3, 's', 'c', 1.0, 'l', 'r', 'a', 'b', 'observe', 0, 0, 'v', 'o')",
                params![id, ts, session],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO prompt_summaries(prompt_id, session_id, ts, model, turns, tools, \
                 output_tokens, thinking_tokens, cost_usd, edited, subagents, simple, \
                 est_saving_usd) VALUES (?1, ?3, ?2, 'm', 1, '', 0, 0, 0, 0, 0, 0, 0)",
                params![id, ts, session],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO suggestions(id, ts, session_id, kind, message) \
                 VALUES (?1, ?2, ?3, 'k', 'm')",
                params![id, ts, session],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO shadow_savings(ts, session_id, kind, key, bytes_saved, \
                 est_tokens_saved, rules_version) VALUES (?1, ?2, 'k', ?3, 0, 0, 'v')",
                params![ts, session, id],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO stored_results(id, session_id, tool, original_hash, content, lines, \
                 bytes, created_at, expires_at) VALUES (?1, ?3, 'Bash', ?1, x'00', 1, 1, ?2, ?2)",
                params![id, ts, session],
            )
            .unwrap();
        self.conn
            .execute(
                "INSERT INTO compressed_blocks(session_id, original_hash, replacement, \
                 rules_version, ts) VALUES (?3, ?1, 'r', 'v', ?2)",
                params![id, ts, session],
            )
            .unwrap();
    }
}

const TABLES: [&str; 7] = [
    "proxy_requests",
    "routing_decisions",
    "prompt_summaries",
    "suggestions",
    "shadow_savings",
    "stored_results",
    "compressed_blocks",
];

#[test]
fn every_table_is_pruned_by_age() {
    let db = open();
    db.seed("old", "gone", NOW - 91 * DAY);
    db.seed("edge", "edge", NOW - 90 * DAY);
    db.seed("new", "recent", NOW - DAY);

    let report = db.store.prune_proxy_data(Timestamp(NOW), 90).unwrap();
    assert_eq!(
        report,
        PruneReport {
            requests: 1,
            decisions: 1,
            prompts: 1,
            suggestions: 1,
            shadow: 1,
            stored_results: 1,
            compressed_blocks: 1,
        }
    );
    assert_eq!(report.total(), 7);
    for table in TABLES {
        assert_eq!(db.count(table), 2, "{table}");
    }
    let again = db.store.prune_proxy_data(Timestamp(NOW), 90).unwrap();
    assert_eq!(again, PruneReport::default());
}

#[test]
fn zero_days_keeps_everything() {
    let db = open();
    db.seed("ancient", "s", 1);
    let report = db.store.prune_proxy_data(Timestamp(NOW), 0).unwrap();
    assert_eq!(report.total(), 0);
    for table in TABLES {
        assert_eq!(db.count(table), 1, "{table}");
    }
}

#[test]
fn a_conversation_with_a_recent_request_keeps_its_results_and_replacements() {
    let db = open();
    // Old tool results and replacements of a conversation that was used a day ago.
    db.seed("old-a", "live", NOW - 120 * DAY);
    db.seed("recent", "live", NOW - DAY);
    db.seed("old-b", "dead", NOW - 120 * DAY);

    let report = db.store.prune_proxy_data(Timestamp(NOW), 90).unwrap();
    assert_eq!(report.requests, 2);
    assert_eq!(report.stored_results, 1, "only the dead conversation's");
    assert_eq!(report.compressed_blocks, 1);
    assert_eq!(db.count("stored_results"), 2);
    assert_eq!(db.count("compressed_blocks"), 2);
    assert_eq!(db.count("proxy_requests"), 1);
}

#[test]
fn a_failing_database_is_reported() {
    let db = open();
    db.conn.execute_batch("DROP TABLE suggestions;").unwrap();
    assert!(db.store.prune_proxy_data(Timestamp(NOW), 90).is_err());
}
