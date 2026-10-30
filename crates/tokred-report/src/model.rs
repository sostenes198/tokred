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
    /// Section 1: summary cards over the whole period (`None` = no data yet).
    pub summary: Option<Summary>,
    /// The same cards over the last 7 days, when the toolbar can switch to them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_7d: Option<Summary>,
    /// The same cards over the last 30 days, when the toolbar can switch to them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_30d: Option<Summary>,
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
    /// What the proxy measured (`None` = the proxy recorded nothing: the report is then exactly
    /// the one without it, and nothing about the proxy is rendered or serialized).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyReport>,
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
    /// How far below an "everything on opus" run the models in use put the cost. It comes from
    /// the models the user runs, not from tokred.
    #[serde(default)]
    pub model_choice_saving_usd: f64,
    /// What tokred's routing saved: the estimated saving of the decisions that were applied.
    pub routing_saving_usd: f64,
    /// Saving from the guards (an estimate, derived from bytes).
    pub guard_saving_usd: f64,
    /// Tokens avoided by the guards (an estimate).
    pub tokens_avoided: u64,
    /// Monthly cost projection (an estimate).
    pub monthly_projection_usd: f64,
    /// Total latency tokred added to the session, in milliseconds.
    pub latency_added_ms: f64,
    /// Hook executions behind `latency_added_ms`.
    #[serde(default)]
    pub hook_runs: u64,
    /// First day these cards cover (`YYYY-MM-DD`), shown when the toolbar switches them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
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
    /// Fable models.
    Fable,
    /// Mythos models.
    Mythos,
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

/// What only the proxy knows: the sections after the audit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProxyReport {
    /// Messages the proxy measured exactly.
    pub measured_messages: u64,
    /// Messages known only from the transcripts (made while the proxy was off).
    pub transcript_only_messages: u64,
    /// Requested against effective model, most requests first.
    pub routes: Vec<ProxyRoute>,
    /// What routing decided in the period.
    pub routing: Option<ProxyRouting>,
    /// The proxy's own overhead.
    pub overhead: Option<ProxyOverhead>,
    /// Main-loop messages in the period (the base of the cache miss rate).
    pub main_requests: u64,
    /// Prompt-cache misses per likely cause, most frequent first.
    pub cache_misses: Vec<ProxyMiss>,
    /// Cost per request origin (`main`, `subagent`, ...).
    pub by_origin: Vec<ProxyBreakdown>,
    /// Cost per agent type of a subagent request.
    pub by_agent_type: Vec<ProxyBreakdown>,
    /// The suggestion waiting to be read.
    pub suggestion: Option<String>,
}

/// One pair of requested and effective model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProxyRoute {
    /// The model Claude Code asked for.
    pub requested: String,
    /// The model that answered.
    pub effective: String,
    /// Messages.
    pub requests: u64,
    /// What they cost on the model that answered, in USD.
    pub cost_usd: f64,
}

/// What routing decided.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProxyRouting {
    /// Decisions where the model was changed.
    pub applied: u64,
    /// Decisions where the model would have been changed (`observe`).
    pub observed: u64,
    /// Decisions that kept the model.
    pub kept: u64,
    /// Rewrites that were rejected and fell back to the original.
    pub fallback: u64,
    /// Estimated saving of the applied decisions (an estimate).
    pub applied_saving_usd: f64,
    /// Estimated saving of the observed decisions, had they been applied (an estimate).
    pub observed_saving_usd: f64,
}

/// The time the proxy added to requests.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProxyOverhead {
    /// Requests measured.
    pub requests: u64,
    /// Median, ms.
    pub p50_ms: f64,
    /// 95th percentile, ms.
    pub p95_ms: f64,
    /// 99th percentile, ms.
    pub p99_ms: f64,
    /// Slowest, ms.
    pub max_ms: f64,
    /// Budget for the p95, ms.
    pub budget_ms: f64,
}

/// Prompt-cache misses of one likely cause.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyMiss {
    /// The cause.
    pub cause: String,
    /// Requests that missed for it.
    pub requests: u64,
    /// Tokens written to the cache again by those requests.
    pub cache_write_tokens: u64,
}

/// Cost of one origin or agent type.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProxyBreakdown {
    /// `main`, `subagent`, `Explore`, ...
    pub name: String,
    /// Messages.
    pub messages: u64,
    /// Tokens.
    pub tokens: u64,
    /// Cost in USD.
    pub cost_usd: f64,
}
