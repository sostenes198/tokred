#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! `prompt_summaries`, `suggestions` and the main-loop activity read from `proxy_requests`.

use rusqlite::{Connection, params};
use tokred_core::Timestamp;
use tokred_core::prompts::{
    MainLoopActivity, PromptRepo, PromptSummary, SuggestionRecord, SuggestionRepo,
};
use tokred_store::Store;

fn prompt(id: &str, session: &str, ts: i64) -> PromptSummary {
    PromptSummary {
        prompt_id: id.to_owned(),
        session_id: session.to_owned(),
        ts: Timestamp(ts),
        model: "claude-opus-5-5".to_owned(),
        turns: 3,
        tools: vec!["Read".to_owned(), "Edit".to_owned()],
        output_tokens: 1_200,
        thinking_tokens: 300,
        cost_usd: 0.25,
        edited: true,
        subagents: 1,
        simple: false,
        est_saving_usd: -0.5,
        detail: r#"{"closed_by":"next_prompt"}"#.to_owned(),
    }
}

fn suggestion(id: &str, session: &str, ts: i64) -> SuggestionRecord {
    SuggestionRecord {
        id: id.to_owned(),
        ts: Timestamp(ts),
        session_id: session.to_owned(),
        kind: "cheaper_model".to_owned(),
        message: format!("tokred: {id}"),
        shown: false,
        detail: r#"{"prompts":3}"#.to_owned(),
    }
}

#[test]
fn prompt_summaries_are_upserted_and_listed_by_time_and_session() {
    let store = Store::open_in_memory().unwrap();
    let first = prompt("s1:100:1", "s1", 100);
    store.upsert_prompt(&first).unwrap();
    store.upsert_prompt(&prompt("s2:150:1", "s2", 150)).unwrap();
    store.upsert_prompt(&prompt("s1:200:2", "s1", 200)).unwrap();

    let all = store
        .prompts_between(Timestamp(0), Timestamp(1_000))
        .unwrap();
    assert_eq!(
        all.iter().map(|p| p.prompt_id.as_str()).collect::<Vec<_>>(),
        ["s1:100:1", "s2:150:1", "s1:200:2"]
    );
    assert_eq!(all[0], first, "every field survives the round trip");

    // `[from, to)`.
    let window = store
        .prompts_between(Timestamp(100), Timestamp(200))
        .unwrap();
    assert_eq!(window.len(), 2);
    let s1 = store
        .session_prompts("s1", Timestamp(0), Timestamp(1_000))
        .unwrap();
    assert_eq!(s1.len(), 2);
    assert!(s1.iter().all(|p| p.session_id == "s1"));
    assert!(
        store
            .session_prompts("nobody", Timestamp(0), Timestamp(1_000))
            .unwrap()
            .is_empty()
    );

    // A prompt written again (closed by the idle flush, then continued) replaces its row.
    let continued = PromptSummary {
        turns: 5,
        tools: Vec::new(),
        simple: true,
        edited: false,
        est_saving_usd: 0.1,
        detail: "not an object".to_owned(),
        ..first
    };
    store.upsert_prompt(&continued).unwrap();
    let rows = store
        .session_prompts("s1", Timestamp(100), Timestamp(101))
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].turns, 5);
    assert!(rows[0].tools.is_empty() && rows[0].simple && !rows[0].edited);
    assert_eq!(
        rows[0].detail, "{}",
        "a detail that is not an object is stored as {{}}"
    );
}

#[test]
fn suggestions_are_stored_once_and_shown_once() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.latest_unshown_suggestion("s1").unwrap().is_none());
    store
        .insert_suggestion(&suggestion("a", "s1", 100))
        .unwrap();
    store
        .insert_suggestion(&suggestion("b", "s1", 200))
        .unwrap();
    store
        .insert_suggestion(&suggestion("c", "s2", 300))
        .unwrap();

    let latest = store.latest_unshown_suggestion("s1").unwrap().unwrap();
    assert_eq!(latest, suggestion("b", "s1", 200));

    assert!(store.mark_suggestion_shown("b").unwrap());
    assert!(
        !store.mark_suggestion_shown("b").unwrap(),
        "only the first caller wins"
    );
    assert!(!store.mark_suggestion_shown("missing").unwrap());
    assert_eq!(
        store.latest_unshown_suggestion("s1").unwrap().unwrap().id,
        "a"
    );

    // Inserting the same id again keeps the stored row (and its shown flag).
    let again = SuggestionRecord {
        message: "changed".to_owned(),
        ..suggestion("b", "s1", 200)
    };
    store.insert_suggestion(&again).unwrap();
    let listed = store
        .suggestions_between(Timestamp(0), Timestamp(1_000))
        .unwrap();
    assert_eq!(
        listed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert!(listed[1].shown);
    assert_eq!(listed[1].message, "tokred: b");
    assert_eq!(
        store
            .suggestions_between(Timestamp(200), Timestamp(300))
            .unwrap()
            .len(),
        1
    );

    let odd = SuggestionRecord {
        detail: String::new(),
        ..suggestion("d", "s3", 400)
    };
    store.insert_suggestion(&odd).unwrap();
    assert_eq!(
        store
            .latest_unshown_suggestion("s3")
            .unwrap()
            .unwrap()
            .detail,
        "{}"
    );
}

#[allow(clippy::too_many_arguments)]
fn request(
    conn: &Connection,
    id: &str,
    session: &str,
    ts: i64,
    origin: &str,
    status: u16,
    write_1h: i64,
) {
    conn.execute(
        "INSERT INTO proxy_requests(id, ts, session_id, origin, model_requested, \
         model_effective, input, output, cache_write, cache_write_1h, cache_read, thinking, \
         bytes_in, bytes_out, upstream_ttfb_ms, total_ms, proxy_overhead_us, status, tools_hash, \
         system_hash) VALUES (?1, ?2, ?3, ?4, 'm', 'm', 0, 0, ?5, ?5, 0, 0, 0, 0, 0, 0, 0, ?6, \
         'h', 'h')",
        params![id, ts, session, origin, write_1h, status],
    )
    .unwrap();
}

#[test]
fn main_loop_activity_comes_from_successful_main_requests() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tokred.db");
    let store = Store::open(&db).unwrap();
    let conn = Connection::open(&db).unwrap();

    assert_eq!(store.main_loop_activity("s1").unwrap(), None);
    request(&conn, "r1", "s1", 100, "main", 200, 0);
    request(&conn, "r2", "s1", 150, "main", 200, 0);
    // Not main-loop activity: a subagent, an auxiliary request, a rejected request.
    request(&conn, "r3", "s1", 400, "subagent", 200, 5);
    request(&conn, "r4", "s1", 410, "auxiliary", 200, 0);
    request(&conn, "r5", "s1", 420, "main", 429, 0);
    request(&conn, "r6", "s2", 500, "main", 200, 0);
    assert_eq!(
        store.main_loop_activity("s1").unwrap(),
        Some(MainLoopActivity {
            last_request: Timestamp(150),
            one_hour_cache: false
        })
    );

    // One 1-hour write by the main loop makes the whole session's cache last an hour.
    request(&conn, "r7", "s1", 120, "main", 200, 3_000);
    let activity = store.main_loop_activity("s1").unwrap().unwrap();
    assert_eq!(activity.last_request, Timestamp(150));
    assert!(activity.one_hour_cache);
    assert_eq!(store.main_loop_activity("s3").unwrap(), None);
}

#[test]
fn a_broken_database_reports_errors() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tokred.db");
    let store = Store::open(&db).unwrap();
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "DROP TABLE prompt_summaries; DROP TABLE suggestions; DROP TABLE proxy_requests;",
    )
    .unwrap();
    assert!(store.upsert_prompt(&prompt("p", "s", 1)).is_err());
    assert!(store.prompts_between(Timestamp(0), Timestamp(9)).is_err());
    assert!(
        store
            .session_prompts("s", Timestamp(0), Timestamp(9))
            .is_err()
    );
    assert!(store.insert_suggestion(&suggestion("a", "s", 1)).is_err());
    assert!(
        store
            .suggestions_between(Timestamp(0), Timestamp(9))
            .is_err()
    );
    assert!(store.latest_unshown_suggestion("s").is_err());
    assert!(store.mark_suggestion_shown("a").is_err());
    assert!(store.main_loop_activity("s").is_err());
}

#[test]
fn damaged_rows_still_read() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("tokred.db");
    let store = Store::open(&db).unwrap();
    let conn = Connection::open(&db).unwrap();
    conn.execute(
        "INSERT INTO prompt_summaries VALUES ('p', 's', 5, 'm', -3, 'oops', -1, 7, 0.5, 0, \
         99999999999, 1, 0.0, '{}')",
        [],
    )
    .unwrap();
    let rows = store.prompts_between(Timestamp(0), Timestamp(9)).unwrap();
    assert_eq!(rows.len(), 1);
    let p = &rows[0];
    assert_eq!(
        (p.turns, p.subagents),
        (0, 0),
        "out-of-range counts read as 0"
    );
    assert!(p.tools.is_empty());
    assert_eq!((p.output_tokens, p.thinking_tokens), (0, 7));
    assert!(p.simple && !p.edited);
}
