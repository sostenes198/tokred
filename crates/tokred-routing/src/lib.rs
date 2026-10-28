//! Model routing policy (spec section 5): classifies subagent and auxiliary requests and picks
//! a cheaper model when it is safe and worth it. The main loop's model is never changed.
//!
//! * [`RoutingHook`] is the proxy step ([`tokred_core::proxy::ProxyHook`]). Built from the
//!   `[routing]` config, the price table and a [`tokred_core::proxy::RecordSink`]; every
//!   decision is sent to the sink as a [`tokred_core::proxy::ProxyRecord::Decision`].
//! * A subagent is decided once, on its first request, and the decision is reused for all its
//!   requests. An auxiliary request is decided on its own, only when moving it pays.
//! * A target is used only when it is cheaper, allowed, and supports everything the request
//!   uses ([`Catalog`], embedded from `data/models.json`).
//! * [`explain`], [`render_status`] and [`summarize_decisions`] produce the CLI text.

mod breaker;
mod catalog;
mod classify;
mod compat;
mod hook;
mod lru;
mod report;

pub use breaker::{BreakerStatus, WINDOW as BREAKER_WINDOW, parse_duration};
pub use catalog::{Catalog, CatalogError, DefaultThinking, ModelInfo};
pub use classify::{
    Classification, Layer, RouteClass, classify_auxiliary, classify_subagent, first_user_text,
};
pub use hook::{RoutingHook, RoutingStatus};
pub use report::{explain, render_status, summarize_decisions};

/// Version of the routing rules, stored with every decision (with the catalog version).
pub const RULES_VERSION: &str = "routing-1";
