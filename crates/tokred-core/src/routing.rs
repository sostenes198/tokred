//! Model routing for subagents and auxiliary requests (spec section 5). The main loop's model is
//! never changed: see [`crate::proxy::rewrite_ticket`].
//!
//! This module holds the data shared with the store and the CLI; the policy itself lives in the
//! `tokred-routing` crate.

use serde::Serialize;

use crate::error::StoreError;
use crate::types::Timestamp;

/// One routing decision (row of `routing_decisions`). Fields not listed here go in `detail`.
///
/// A decision can be recorded more than once under the same `id`: a subagent's sticky
/// decision is re-sent with outcome `fallback` when its rewrite stops. The last version wins.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RoutingDecision {
    /// Unique id.
    pub id: String,
    /// When it was taken.
    pub ts: Timestamp,
    /// Session id.
    pub session_id: String,
    /// Agent id for a subagent decision (`None` for a single auxiliary request).
    pub agent_id: Option<String>,
    /// `agent` (sticky for the whole subagent) or `request`.
    pub scope: String,
    /// The task class (`search`, `summary`, `implementation`, `planning`, `architecture`,
    /// `review`, `auxiliary`, `other`).
    pub task_class: String,
    /// Classification confidence, 0..=1.
    pub confidence: f64,
    /// Which classification layer decided (`headers`, `explicit`, `tools`, `keywords`,
    /// `shape` for the auxiliary request shape, `mid_run` for an agent first seen mid-run,
    /// `default`).
    pub layer: String,
    /// Human-readable reason.
    pub reason: String,
    /// The model that was requested.
    pub model_from: String,
    /// The model chosen (equal to `model_from` when kept).
    pub model_to: String,
    /// The routing mode in effect (`observe` or `enforce`).
    pub mode: String,
    /// Estimated cost of the request on `model_from`, USD.
    pub est_cost_from: f64,
    /// Estimated cost of the request on `model_to`, USD.
    pub est_cost_to: f64,
    /// Version of the routing rules.
    pub rules_version: String,
    /// `applied`, `observed`, `kept` or `fallback`.
    pub outcome: String,
    /// Extra fields as a JSON object (`guard`: what kept the model; `tier`; `candidate` and
    /// `candidate_est_cost` when a target was found but not used; `stopped` and
    /// `original_reads_target_thinking` when a rewrite stopped; `input_tokens_est`).
    pub detail: String,
}

/// Persistent routing decisions (`routing_decisions`).
pub trait RoutingRepo {
    /// Stores a decision, replacing an earlier version with the same id.
    fn insert_decision(&self, decision: &RoutingDecision) -> Result<(), StoreError>;
    /// The decision with this id.
    fn decision(&self, id: &str) -> Result<Option<RoutingDecision>, StoreError>;
    /// Decisions with `from <= ts < to`, oldest first.
    fn decisions_between(
        &self,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<RoutingDecision>, StoreError>;
    /// Decisions of one subagent, oldest first.
    fn decisions_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Vec<RoutingDecision>, StoreError>;
    /// The latest `limit` decisions, newest first.
    fn recent_decisions(&self, limit: usize) -> Result<Vec<RoutingDecision>, StoreError>;
}
