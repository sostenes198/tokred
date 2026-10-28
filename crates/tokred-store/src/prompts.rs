//! `prompt_summaries` and `suggestions` repositories (suggestions workstream), plus the one read
//! of `proxy_requests` the `UserPromptSubmit` hook needs to tell whether the cache is cold.

use rusqlite::{OptionalExtension, Row, params};
use tokred_core::prompts::{
    MainLoopActivity, PromptRepo, PromptSummary, SuggestionRecord, SuggestionRepo, decode_tools,
    encode_tools, normalize_detail,
};
use tokred_core::{StoreError, Timestamp};

use crate::Store;
use crate::session::{from_db, to_db};

const PROMPT_COLUMNS: &str = "prompt_id, session_id, ts, model, turns, tools, output_tokens, \
     thinking_tokens, cost_usd, edited, subagents, simple, est_saving_usd, detail";

const SUGGESTION_COLUMNS: &str = "id, ts, session_id, kind, message, shown, detail";

fn small(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

fn prompt_from_row(row: &Row<'_>) -> rusqlite::Result<PromptSummary> {
    Ok(PromptSummary {
        prompt_id: row.get(0)?,
        session_id: row.get(1)?,
        ts: Timestamp(row.get(2)?),
        model: row.get(3)?,
        turns: small(row.get(4)?),
        tools: decode_tools(&row.get::<_, String>(5)?),
        output_tokens: from_db(row.get(6)?),
        thinking_tokens: from_db(row.get(7)?),
        cost_usd: row.get(8)?,
        edited: row.get(9)?,
        subagents: small(row.get(10)?),
        simple: row.get(11)?,
        est_saving_usd: row.get(12)?,
        detail: row.get(13)?,
    })
}

fn suggestion_from_row(row: &Row<'_>) -> rusqlite::Result<SuggestionRecord> {
    Ok(SuggestionRecord {
        id: row.get(0)?,
        ts: Timestamp(row.get(1)?),
        session_id: row.get(2)?,
        kind: row.get(3)?,
        message: row.get(4)?,
        shown: row.get(5)?,
        detail: row.get(6)?,
    })
}

impl Store {
    fn query_prompts(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<PromptSummary>, StoreError> {
        let mut stmt = self.conn.prepare(sql).map_err(StoreError::backend)?;
        let rows = stmt
            .query_map(params, prompt_from_row)
            .map_err(StoreError::backend)?;
        rows.collect::<Result<_, _>>().map_err(StoreError::backend)
    }
}

impl PromptRepo for Store {
    fn upsert_prompt(&self, summary: &PromptSummary) -> Result<(), StoreError> {
        self.conn
            .execute(
                &format!(
                    "INSERT INTO prompt_summaries({PROMPT_COLUMNS}) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) \
                     ON CONFLICT(prompt_id) DO UPDATE SET session_id = excluded.session_id, \
                     ts = excluded.ts, model = excluded.model, turns = excluded.turns, \
                     tools = excluded.tools, output_tokens = excluded.output_tokens, \
                     thinking_tokens = excluded.thinking_tokens, cost_usd = excluded.cost_usd, \
                     edited = excluded.edited, subagents = excluded.subagents, \
                     simple = excluded.simple, est_saving_usd = excluded.est_saving_usd, \
                     detail = excluded.detail"
                ),
                params![
                    summary.prompt_id,
                    summary.session_id,
                    summary.ts.0,
                    summary.model,
                    i64::from(summary.turns),
                    encode_tools(&summary.tools),
                    to_db(summary.output_tokens),
                    to_db(summary.thinking_tokens),
                    summary.cost_usd,
                    summary.edited,
                    i64::from(summary.subagents),
                    summary.simple,
                    summary.est_saving_usd,
                    normalize_detail(&summary.detail),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }

    fn prompts_between(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<PromptSummary>, StoreError> {
        self.query_prompts(
            &format!(
                "SELECT {PROMPT_COLUMNS} FROM prompt_summaries \
                 WHERE ts >= ?1 AND ts < ?2 ORDER BY ts, prompt_id"
            ),
            params![from.0, to.0],
        )
    }

    fn session_prompts(
        &self,
        session_id: &str,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<PromptSummary>, StoreError> {
        self.query_prompts(
            &format!(
                "SELECT {PROMPT_COLUMNS} FROM prompt_summaries \
                 WHERE session_id = ?1 AND ts >= ?2 AND ts < ?3 ORDER BY ts, prompt_id"
            ),
            params![session_id, from.0, to.0],
        )
    }
}

impl SuggestionRepo for Store {
    fn insert_suggestion(&self, suggestion: &SuggestionRecord) -> Result<(), StoreError> {
        self.conn
            .execute(
                &format!(
                    "INSERT INTO suggestions({SUGGESTION_COLUMNS}) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(id) DO NOTHING"
                ),
                params![
                    suggestion.id,
                    suggestion.ts.0,
                    suggestion.session_id,
                    suggestion.kind,
                    suggestion.message,
                    suggestion.shown,
                    normalize_detail(&suggestion.detail),
                ],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }

    fn suggestions_between(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<SuggestionRecord>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {SUGGESTION_COLUMNS} FROM suggestions \
                 WHERE ts >= ?1 AND ts < ?2 ORDER BY ts, id"
            ))
            .map_err(StoreError::backend)?;
        let rows = stmt
            .query_map(params![from.0, to.0], suggestion_from_row)
            .map_err(StoreError::backend)?;
        rows.collect::<Result<_, _>>().map_err(StoreError::backend)
    }

    fn latest_unshown_suggestion(
        &self,
        session_id: &str,
    ) -> Result<Option<SuggestionRecord>, StoreError> {
        self.conn
            .query_row(
                &format!(
                    "SELECT {SUGGESTION_COLUMNS} FROM suggestions \
                     WHERE session_id = ?1 AND shown = 0 ORDER BY ts DESC, id DESC LIMIT 1"
                ),
                params![session_id],
                suggestion_from_row,
            )
            .optional()
            .map_err(StoreError::backend)
    }

    fn mark_suggestion_shown(&self, id: &str) -> Result<bool, StoreError> {
        self.conn
            .execute(
                "UPDATE suggestions SET shown = 1 WHERE id = ?1 AND shown = 0",
                params![id],
            )
            .map(|changed| changed == 1)
            .map_err(StoreError::backend)
    }

    fn main_loop_activity(&self, session_id: &str) -> Result<Option<MainLoopActivity>, StoreError> {
        self.conn
            .query_row(
                "SELECT MAX(ts), COALESCE(MAX(cache_write_1h > 0), 0) FROM proxy_requests \
                 WHERE session_id = ?1 AND origin = 'main' AND status BETWEEN 200 AND 299",
                params![session_id],
                |row| {
                    let last: Option<i64> = row.get(0)?;
                    let one_hour_cache: bool = row.get(1)?;
                    Ok(last.map(|last| MainLoopActivity {
                        last_request: Timestamp(last),
                        one_hour_cache,
                    }))
                },
            )
            .map_err(StoreError::backend)
    }
}
