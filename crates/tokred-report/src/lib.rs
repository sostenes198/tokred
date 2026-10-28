//! HTML metrics report generation for tokred (`tokred metrics`).
//!
//! The crate turns a plain-data [`ReportData`] into a single self-contained, offline
//! HTML page ([`render_html`]): inline CSS and JS, SVG charts generated in Rust, no
//! CDN, no external font and no network request. [`write_report`] writes it
//! atomically through the [`tokred_core::FileSystem`] port and [`open::Opener`] opens
//! it in the browser.
//!
//! Nothing here reads a database or a transcript: the CLI maps the metrics results
//! into [`ReportData`] and composes the ports.

// HTML and SVG fragments are assembled from many small `format!` calls; the clarity is
// worth the extra short-lived allocations for a file rendered once per command.
#![allow(clippy::format_push_string, clippy::format_collect)]

pub mod escape;
pub mod format;
pub mod model;
pub mod open;
mod render;
mod sections;
pub mod svg;
#[cfg(test)]
mod testutil;
mod write;

pub use model::{
    AuditItem, AuditStats, DayCache, DayCost, GuardReason, GuardStat, HookLatency, LatencyStats,
    ModelFamily, ModelUsage, ReportData, ReportMeta, RoutingStats, Summary, TaskTypeCount,
};
pub use open::{Opener, SystemOpener};
pub use render::render_html;
pub use write::{ReportError, write_report};
