//! Fixed-context audit behind `tokred audit`.
//!
//! The audit weighs what Claude Code loads before the first prompt (user and project `CLAUDE.md`
//! files with their `@path` imports, rules, skills and agents) and compares the configured MCP
//! servers with the calls found in the transcripts. Token counts are estimates (characters / 4).
//!
//! The logic is pure over ports: [`tokred_core::FileSystem`] for reads, plus [`DirLister`] and
//! [`McpUsageSource`] defined here. The [`fs`] module holds the `std` adapters.

mod audit;
mod frontmatter;
mod mcp;
mod memory;

pub mod fs;
pub mod ports;
pub mod report;

pub use audit::{AuditRequest, DEFAULT_WINDOW_DAYS, Ports, audit, global_config_candidates};
pub use ports::{DirEntry, DirLister, McpUsage, McpUsageSource, UsageError, UsageScan};
pub use report::{
    AuditItem, AuditReport, ItemKind, KindTotal, McpScope, McpServerReport, Scope, Summary,
    UnconfiguredUsage,
};
