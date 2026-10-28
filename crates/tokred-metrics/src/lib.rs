//! Transcript ingestion, cost, savings and latency metrics (the `stats` use cases).
//!
//! Everything here is a pure function over data, except [`fs::StdTranscriptFs`], the one adapter
//! that touches the disk. I/O reaches the use cases through ports received as `&dyn` parameters:
//! [`TranscriptFs`] for transcripts and [`tokred_core::usage::UsageRepo`] /
//! [`tokred_core::usage::IngestRepo`] for the database.
//!
//! * [`ingest()`] reads `<claude>/projects/**/*.jsonl` incrementally into `usage`.
//! * [`compute_stats`] turns usage rows, prices and hook events into cost, savings, cache rate,
//!   projection and before/after numbers.
//! * [`latency_stats`] summarizes how long the hooks delayed Claude Code.
//! * [`routing_summary`] and [`guards_summary`] feed the "Routing" and "Guards" report sections.
//!
//! Amounts derived from bytes (guard savings) are estimates and carry `estimate: true`.

pub mod error;
pub mod fs;
pub mod ingest;
pub mod latency;
pub mod report;
pub mod span;
pub mod stats;
pub mod transcript;
mod util;

pub use error::MetricsError;
pub use fs::StdTranscriptFs;
pub use ingest::{IngestSummary, TranscriptFile, TranscriptFs, ingest};
pub use latency::{LatencyStats, Tradeoff, latency_stats, tradeoff};
pub use report::{GuardsSummary, RoutingSummary, guards_summary, routing_summary};
pub use span::{SpanError, TimeRange, parse_span, range_from_span};
pub use stats::{GroupBy, Stats, StatsInput, compute_stats};
pub use transcript::{LineContext, parse_line};
