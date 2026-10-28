//! Model routing for subagents and auxiliary requests (spec section 5). The main loop's model is
//! never changed: see [`crate::proxy::rewrite_ticket`].
//!
//! This module holds the data shared with the store and the CLI; the policy itself lives in the
//! `tokred-routing` crate.

use serde::Serialize;

use crate::types::Timestamp;

/// One routing decision (row of `routing_decisions`). Fields not listed here go in `detail`.
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
    /// Which classification layer decided (`headers`, `explicit`, `tools`, `keywords`, `default`).
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
    /// Extra fields as a JSON object.
    pub detail: String,
}
