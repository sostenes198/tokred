//! `routing_decisions` repository (routing workstream).

use rusqlite::{OptionalExtension, params};
use tokred_core::routing::{RoutingDecision, RoutingRepo};
use tokred_core::{StoreError, Timestamp};

use crate::Store;

const SELECT: &str = "SELECT id, ts, session_id, agent_id, scope, task_class, confidence, layer, \
                      reason, model_from, model_to, mode, est_cost_from, est_cost_to, \
                      rules_version, outcome, detail FROM routing_decisions";

fn row_to_decision(row: &rusqlite::Row<'_>) -> rusqlite::Result<RoutingDecision> {
    Ok(RoutingDecision {
        id: row.get(0)?,
        ts: Timestamp(row.get(1)?),
        session_id: row.get(2)?,
        agent_id: row.get(3)?,
        scope: row.get(4)?,
        task_class: row.get(5)?,
        confidence: row.get(6)?,
        layer: row.get(7)?,
        reason: row.get(8)?,
        model_from: row.get(9)?,
        model_to: row.get(10)?,
        mode: row.get(11)?,
        est_cost_from: row.get(12)?,
        est_cost_to: row.get(13)?,
        rules_version: row.get(14)?,
        outcome: row.get(15)?,
        detail: row.get(16)?,
    })
}

impl Store {
    fn query_decisions(
        &self,
        clause: &str,
        params: impl rusqlite::Params,
    ) -> Result<Vec<RoutingDecision>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(&format!("{SELECT} {clause}"))
            .map_err(StoreError::backend)?;
        stmt.query_map(params, row_to_decision)
            .map_err(StoreError::backend)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::backend)
    }
}

impl RoutingRepo for Store {
    fn insert_decision(&self, decision: &RoutingDecision) -> Result<(), StoreError> {
        // An empty `detail` is stored as the empty JSON object.
        let detail = if decision.detail.is_empty() {
            "{}"
        } else {
            decision.detail.as_str()
        };
        self.conn
            .execute(
                "INSERT INTO routing_decisions(id, ts, session_id, agent_id, scope, task_class, \
                 confidence, layer, reason, model_from, model_to, mode, est_cost_from, \
                 est_cost_to, rules_version, outcome, detail) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17) \
                 ON CONFLICT(id) DO UPDATE SET ts = excluded.ts, session_id = excluded.session_id, \
                 agent_id = excluded.agent_id, scope = excluded.scope, \
                 task_class = excluded.task_class, confidence = excluded.confidence, \
                 layer = excluded.layer, reason = excluded.reason, \
                 model_from = excluded.model_from, model_to = excluded.model_to, \
                 mode = excluded.mode, est_cost_from = excluded.est_cost_from, \
                 est_cost_to = excluded.est_cost_to, rules_version = excluded.rules_version, \
                 outcome = excluded.outcome, detail = excluded.detail",
                params![
                    decision.id,
                    decision.ts.0,
                    decision.session_id,
                    decision.agent_id,
                    decision.scope,
                    decision.task_class,
                    decision.confidence,
                    decision.layer,
                    decision.reason,
                    decision.model_from,
                    decision.model_to,
                    decision.mode,
                    decision.est_cost_from,
                    decision.est_cost_to,
                    decision.rules_version,
                    decision.outcome,
                    detail,
                ],
            )
            .map(|_| ())
            .map_err(StoreError::backend)
    }

    fn decision(&self, id: &str) -> Result<Option<RoutingDecision>, StoreError> {
        self.conn
            .query_row(
                &format!("{SELECT} WHERE id = ?1"),
                params![id],
                row_to_decision,
            )
            .optional()
            .map_err(StoreError::backend)
    }

    fn decisions_between(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<RoutingDecision>, StoreError> {
        self.query_decisions(
            "WHERE ts >= ?1 AND ts < ?2 ORDER BY ts, rowid",
            params![from.0, to.0],
        )
    }

    fn decisions_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<RoutingDecision>, StoreError> {
        self.query_decisions(
            "WHERE session_id = ?1 AND agent_id = ?2 ORDER BY ts, rowid",
            params![session_id, agent_id],
        )
    }

    fn recent_decisions(&self, limit: usize) -> Result<Vec<RoutingDecision>, StoreError> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.query_decisions("ORDER BY ts DESC, rowid DESC LIMIT ?1", params![limit])
    }
}
