//! The plain-data input of the report.
//!
//! Everything here is numbers, strings and vectors so the CLI can fill it from the
//! metrics results without this crate knowing about them. Conventions:
//!
//! - dates are `YYYY-MM-DD` strings; daily series must be sorted ascending and
//!   gap-filled (one entry per calendar day), because the 7/30-day views are the
//!   *last* 7/30 entries;
//! - money is in USD;
//! - values documented as *estimates* are labelled as such in the HTML.

use serde::{Deserialize, Serialize};

/// Everything the HTML report shows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReportData {
    /// Footer information.
    pub meta: ReportMeta,
    /// Section 1: summary cards (`None` = no data yet).
    pub summary: Option<Summary>,
    /// Section 2: cost per day, ascending.
    pub cost_per_day: Vec<DayCost>,
    /// Section 3: usage per model.
    pub models: Vec<ModelUsage>,
    /// Section 4: routing statistics (`None` = no data yet).
    pub routing: Option<RoutingStats>,
    /// Section 5: blocks per guard reason.
    pub guards: Vec<GuardStat>,
    /// Section 6: prompt-cache hit rate per day, ascending.
    pub cache_per_day: Vec<DayCache>,
    /// Section 7: hook latency (`None` = no data yet).
    pub latency: Option<LatencyStats>,
    /// Section 8: audit of the fixed context (`None` = no data yet).
    pub audit: Option<AuditStats>,
}

/// Footer information of the report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportMeta {
    /// When the report was generated (free-form, for example `2026-10-28 14:03 UTC`).
    pub generated_at: String,
    /// The tokred version that generated the report.
    pub tokred_version: String,
    /// First day covered (`YYYY-MM-DD`), when known.
    pub period_start: Option<String>,
    /// Last day covered (`YYYY-MM-DD`), when known.
    pub period_end: Option<String>,
    /// Date of the last price sync, when there was one.
    pub last_price_sync: Option<String>,
}

/// Section 1: the summary cards.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Real cost over the period.
    pub real_cost_usd: f64,
    /// Saving from model routing (real cost versus "everything on opus").
    pub routing_saving_usd: f64,
    /// Saving from the guards (an estimate, derived from bytes).
    pub guard_saving_usd: f64,
    /// Tokens avoided by the guards (an estimate).
    pub tokens_avoided: u64,
    /// Monthly cost projection (an estimate).
    pub monthly_projection_usd: f64,
    /// Total latency tokred added to the session, in milliseconds.
    pub latency_added_ms: f64,
}

/// Cost of one day (section 2).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DayCost {
    /// The day, `YYYY-MM-DD`.
    pub date: String,
    /// What the day really cost.
    pub real_usd: f64,
    /// What the same tokens would have cost on opus.
    pub all_opus_usd: f64,
}

/// Model family, used to colour the distribution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelFamily {
    /// Opus models.
    Opus,
    /// Sonnet models.
    Sonnet,
    /// Haiku models.
    Haiku,
    /// Anything else.
    #[default]
    Other,
}

/// Usage of one model (section 3).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelUsage {
    /// Model name as it appears in the transcripts.
    pub name: String,
    /// Family the model belongs to.
    pub family: ModelFamily,
    /// Tokens processed.
    pub tokens: u64,
    /// Cost in USD.
    pub cost_usd: f64,
}

/// Section 4: routing statistics.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingStats {
    /// Classifications per task type.
    pub classifications: Vec<TaskTypeCount>,
    /// Delegations that already used the right model.
    pub delegations_correct: u64,
    /// Delegations that `enforce` had to correct.
    pub delegations_corrected: u64,
}

/// Number of classifications for one task type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskTypeCount {
    /// The task type (for example `explore`, `edit`, `plan`).
    pub task_type: String,
    /// How many prompts were classified as it.
    pub count: u64,
}

/// Why a guard blocked a call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardReason {
    /// The same file was read again unchanged.
    #[default]
    Reread,
    /// A generated file (lock file, build output).
    GeneratedFile,
    /// A file above the size limit.
    LargeFile,
    /// A search too broad for the repository size.
    BroadSearch,
}

/// Blocks for one guard reason (section 5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardStat {
    /// Why the calls were blocked.
    pub reason: GuardReason,
    /// Number of blocked calls.
    pub blocks: u64,
    /// Bytes the blocked calls would have loaded (an estimate).
    pub bytes_avoided: u64,
    /// Tokens avoided (an estimate, bytes / 4).
    pub tokens_avoided: u64,
}

/// Prompt-cache hit rate of one day (section 6).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DayCache {
    /// The day, `YYYY-MM-DD`.
    pub date: String,
    /// Hit rate as a fraction between 0 and 1.
    pub hit_rate: f64,
}

/// Section 7: latency added by the hooks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LatencyStats {
    /// The per-hook latency budget in milliseconds (50 by default).
    pub budget_ms: f64,
    /// Latency per hook.
    pub hooks: Vec<HookLatency>,
    /// Total latency added, in milliseconds.
    pub total_added_ms: f64,
    /// Money saved in the same period (an estimate), for the trade-off comparison.
    pub saved_usd: f64,
    /// Tokens saved in the same period (an estimate), for the trade-off comparison.
    pub saved_tokens: u64,
    /// Average process spawn overhead from `doctor --bench` (an estimate), when measured.
    pub spawn_overhead_ms: Option<f64>,
}

/// Latency of one hook.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HookLatency {
    /// The hook name (for example `guard-read`).
    pub hook: String,
    /// Number of executions.
    pub runs: u64,
    /// Median duration in milliseconds.
    pub p50_ms: f64,
    /// 95th percentile in milliseconds.
    pub p95_ms: f64,
    /// Maximum in milliseconds.
    pub max_ms: f64,
}

/// Section 8: audit of the fixed context.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditStats {
    /// The heaviest items of the fixed context, heaviest first.
    pub heaviest: Vec<AuditItem>,
    /// MCP servers with no use in the window.
    pub unused_mcp: Vec<String>,
    /// Size of the window used to decide a server is unused, in days.
    pub unused_window_days: Option<u32>,
}

/// One item of the fixed context.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditItem {
    /// Item name (file name, skill, agent).
    pub name: String,
    /// What it is (for example `CLAUDE.md`, `skill`, `agent`).
    pub kind: String,
    /// Where it comes from (a project name, or `user`).
    pub origin: String,
    /// Weight in tokens (an estimate, characters / 4).
    pub tokens: u64,
}
