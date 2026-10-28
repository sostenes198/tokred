//! The `tokred.toml` configuration model and its non-destructive merge with defaults.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

/// Behaviour of a hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// The hook does nothing.
    Off,
    /// The hook allows the action and only advises.
    Warn,
    /// The hook denies the action.
    Enforce,
}

/// `[claude]` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeConfig {
    /// Claude Code folder chosen by a previous `configure`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

/// `[modes]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Modes {
    /// `route` hook mode.
    pub route: Mode,
    /// `enforce` hook mode.
    pub enforce: Mode,
    /// `guard-read` hook mode.
    pub guard_read: Mode,
    /// `guard-search` hook mode.
    pub guard_search: Mode,
    /// `session-check` hook mode.
    pub session_check: Mode,
}

impl Default for Modes {
    fn default() -> Self {
        Self {
            route: Mode::Warn,
            enforce: Mode::Warn,
            guard_read: Mode::Enforce,
            guard_search: Mode::Warn,
            session_check: Mode::Warn,
        }
    }
}

/// `[guard_read]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuardRead {
    /// Files larger than this need `offset`/`limit`.
    pub max_read_bytes: u64,
    /// Globs that are always denied.
    pub deny_globs: Vec<String>,
}

impl Default for GuardRead {
    fn default() -> Self {
        let deny_globs = [
            "**/package-lock.json",
            "**/pnpm-lock.yaml",
            "**/yarn.lock",
            "**/Cargo.lock",
            "**/poetry.lock",
            "**/*.min.js",
            "**/*.map",
            "**/dist/**",
            "**/build/**",
            "**/node_modules/**",
            "**/target/**",
            "**/.git/**",
        ];
        Self {
            max_read_bytes: 204_800,
            deny_globs: deny_globs.iter().map(|g| (*g).to_owned()).collect(),
        }
    }
}

/// `[guard_search]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuardSearch {
    /// Repositories with more files than this count as large.
    pub large_repo_files: u64,
}

impl Default for GuardSearch {
    fn default() -> Self {
        Self {
            large_repo_files: 5000,
        }
    }
}

/// `[session_check]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionCheck {
    /// Context size (tokens) above which the user is warned.
    pub context_warn_tokens: u64,
}

impl Default for SessionCheck {
    fn default() -> Self {
        Self {
            context_warn_tokens: 120_000,
        }
    }
}

/// Update behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateMode {
    /// Never check for new versions.
    Off,
    /// Print a notice when a new version exists.
    Notify,
    /// Self-update (script installs only).
    Auto,
}

/// `[update]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Update {
    /// Update mode.
    pub mode: UpdateMode,
}

impl Default for Update {
    fn default() -> Self {
        Self {
            mode: UpdateMode::Notify,
        }
    }
}

/// `[pricing]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pricing {
    /// Sync prices automatically once a day.
    pub auto_sync: bool,
}

impl Default for Pricing {
    fn default() -> Self {
        Self { auto_sync: true }
    }
}

/// `[cache]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    /// L1 time to live.
    pub l1_ttl: String,
    /// L2 time to live.
    pub l2_ttl: String,
    /// Hits needed to reach L2.
    pub l2_hits: u32,
    /// Hits needed to reach L3.
    pub l3_hits: u32,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            l1_ttl: "4h".to_owned(),
            l2_ttl: "24h".to_owned(),
            l2_hits: 3,
            l3_hits: 10,
        }
    }
}

/// Behaviour of a feature that can alter API traffic through the proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrafficMode {
    /// The feature does nothing.
    Off,
    /// The feature only records what it would do; traffic is never altered.
    Observe,
    /// The feature alters traffic.
    Enforce,
}

/// `[proxy]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxyConfig {
    /// Whether `configure` set the proxy up.
    pub enabled: bool,
    /// Address the proxy listens on (loopback only unless `--bind` says otherwise).
    pub bind: String,
    /// The only host requests are forwarded to.
    pub upstream: String,
    /// Write `env.ANTHROPIC_BASE_URL` into Claude Code's settings (needs the user service).
    pub persist_base_url: bool,
    /// Days of proxy records kept.
    pub retention_days: u32,
    /// Log request bodies (debugging only; never credentials).
    pub log_bodies: bool,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "127.0.0.1:8787".to_owned(),
            upstream: "https://api.anthropic.com".to_owned(),
            persist_base_url: false,
            retention_days: 90,
            log_bodies: false,
        }
    }
}

/// What a task class is routed to: a tier the model catalog resolves to a concrete model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TierChoice {
    /// The cheapest suitable model.
    Cheap,
    /// A mid-priced model.
    Mid,
    /// Keep the model that was requested.
    Keep,
}

/// `[routing.tiers]`: task class -> tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingTiers {
    /// Searching code (Explore-like subagents).
    pub search: TierChoice,
    /// Summarizing.
    pub summary: TierChoice,
    /// Implementing, fixing, refactoring.
    pub implementation: TierChoice,
    /// Planning.
    pub planning: TierChoice,
    /// Architecture and design.
    pub architecture: TierChoice,
    /// Reviewing.
    pub review: TierChoice,
    /// Auxiliary requests (session titles and the like).
    pub auxiliary: TierChoice,
}

impl Default for RoutingTiers {
    fn default() -> Self {
        Self {
            search: TierChoice::Cheap,
            summary: TierChoice::Cheap,
            implementation: TierChoice::Mid,
            planning: TierChoice::Keep,
            architecture: TierChoice::Keep,
            review: TierChoice::Keep,
            auxiliary: TierChoice::Cheap,
        }
    }
}

/// `[routing]` section. The main loop's model is never changed, whatever this says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingConfig {
    /// `off`, `observe` (record only, the default) or `enforce`.
    pub mode: TrafficMode,
    /// Minimum classification confidence (0..=1) to act.
    pub min_confidence: f64,
    /// Minimum estimated saving in USD for one decision.
    pub min_saving_usd: f64,
    /// Rewrite failure rate that opens the circuit breaker.
    pub max_rewrite_failure_rate: f64,
    /// How long the circuit breaker stays open.
    pub cooldown: String,
    /// Models routing may pick; empty means any cheaper model of the catalog.
    pub allowed_models: Vec<String>,
    /// Compaction requests: `keep` (default) or `cheap` (opt-in).
    pub compaction: TierChoice,
    /// Tier per task class.
    pub tiers: RoutingTiers,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            mode: TrafficMode::Observe,
            min_confidence: 0.8,
            min_saving_usd: 0.005,
            max_rewrite_failure_rate: 0.05,
            cooldown: "30m".to_owned(),
            allowed_models: Vec::new(),
            compaction: TierChoice::Keep,
            tiers: RoutingTiers::default(),
        }
    }
}

/// Where model suggestions for the main loop are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SuggestChannel {
    /// Not shown.
    Off,
    /// In `tokred suggest` only.
    Stats,
    /// Also in the status line (`tokred statusline`).
    Statusline,
    /// Also as a one-line message to the user at safe moments.
    Message,
}

/// `[suggest]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SuggestConfig {
    /// Where suggestions appear.
    pub channel: SuggestChannel,
    /// Simple prompts in a row before a suggestion is made.
    pub min_simple_prompts: u32,
    /// Minimum time between two suggestions in a session.
    pub cooldown: String,
}

impl Default for SuggestConfig {
    fn default() -> Self {
        Self {
            channel: SuggestChannel::Stats,
            min_simple_prompts: 3,
            cooldown: "6h".to_owned(),
        }
    }
}

/// `[compression]` section: reversible compression of large tool results (spec 6.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompressionConfig {
    /// `off`, `observe` (measure only, the default) or `enforce`.
    pub mode: TrafficMode,
    /// Only text tool results above this many tokens (bytes / 4) are replaced.
    pub min_tokens: u64,
    /// Lines kept from the start of a replaced block.
    pub preview_head_lines: u32,
    /// Lines kept from the end of a replaced block.
    pub preview_tail_lines: u32,
    /// How long originals are kept for `tokred_retrieve`.
    pub retention: String,
    /// Retrievals of one id in a session before its tool is excluded from compression.
    pub max_retrievals: u32,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            mode: TrafficMode::Observe,
            min_tokens: 1500,
            preview_head_lines: 40,
            preview_tail_lines: 20,
            retention: "7d".to_owned(),
            max_retrievals: 3,
        }
    }
}

/// `[shadow]` section: measurements that never alter traffic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShadowConfig {
    /// Measure what reversible compression would save.
    pub compression: bool,
    /// Measure how many requests are exact repeats.
    pub response_cache: bool,
}

impl Default for ShadowConfig {
    fn default() -> Self {
        Self {
            compression: true,
            response_cache: true,
        }
    }
}

/// The whole `tokred.toml`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `[claude]`.
    pub claude: ClaudeConfig,
    /// `[modes]`.
    pub modes: Modes,
    /// `[guard_read]`.
    pub guard_read: GuardRead,
    /// `[guard_search]`.
    pub guard_search: GuardSearch,
    /// `[session_check]`.
    pub session_check: SessionCheck,
    /// `[update]`.
    pub update: Update,
    /// `[pricing]`.
    pub pricing: Pricing,
    /// `[cache]`.
    pub cache: CacheConfig,
    /// `[proxy]`.
    pub proxy: ProxyConfig,
    /// `[routing]`.
    pub routing: RoutingConfig,
    /// `[suggest]`.
    pub suggest: SuggestConfig,
    /// `[compression]`.
    pub compression: CompressionConfig,
    /// `[shadow]`.
    pub shadow: ShadowConfig,
}

impl Config {
    /// Parses a `tokred.toml`; missing keys take their default value.
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(text)?)
    }
}

/// Merges `existing` (the user's `tokred.toml`, possibly empty) with the defaults.
///
/// New keys are added with their default value; every existing value is preserved.
/// When `claude_dir` is given it is stored in `[claude] dir`.
/// Returns the text to write. When nothing needs to change, returns `existing` untouched
/// (so comments and formatting survive).
pub fn merge_config(existing: &str, claude_dir: Option<&Path>) -> Result<String, ConfigError> {
    let mut table: toml::Table = toml::from_str(existing)?;
    let defaults = toml::Table::try_from(Config::default())?;
    let mut changed = false;
    if let Some(dir) = claude_dir {
        changed |= set_claude_dir(&mut table, dir)?;
    }
    changed |= add_missing(&mut table, &defaults, "")?;

    if changed || existing.trim().is_empty() {
        Ok(toml::to_string_pretty(&table)?)
    } else {
        Ok(existing.to_owned())
    }
}

/// Stores `dir` in `[claude] dir`; returns whether the value changed.
fn set_claude_dir(table: &mut toml::Table, dir: &Path) -> Result<bool, ConfigError> {
    let value = toml::Value::String(dir.to_string_lossy().into_owned());
    let claude = table
        .entry("claude")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| ConfigError::NotATable("claude".to_owned()))?;
    if claude.get("dir") == Some(&value) {
        return Ok(false);
    }
    claude.insert("dir".to_owned(), value);
    Ok(true)
}

fn add_missing(
    target: &mut toml::Table,
    defaults: &toml::Table,
    prefix: &str,
) -> Result<bool, ConfigError> {
    let mut changed = false;
    for (key, default) in defaults {
        match (target.get_mut(key), default) {
            (None, _) => {
                target.insert(key.clone(), default.clone());
                changed = true;
            }
            (Some(toml::Value::Table(existing)), toml::Value::Table(default_table)) => {
                let path = format!("{prefix}{key}.");
                changed |= add_missing(existing, default_table, &path)?;
            }
            (Some(_), toml::Value::Table(_)) => {
                return Err(ConfigError::NotATable(format!("{prefix}{key}")));
            }
            (Some(_), _) => {}
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_spec() {
        let cfg = Config::default();
        assert_eq!(cfg.modes.guard_read, Mode::Enforce);
        assert_eq!(cfg.modes.route, Mode::Warn);
        assert_eq!(cfg.guard_read.max_read_bytes, 204_800);
        assert_eq!(cfg.guard_search.large_repo_files, 5000);
        assert_eq!(cfg.session_check.context_warn_tokens, 120_000);
        assert_eq!(cfg.update.mode, UpdateMode::Notify);
        assert!(cfg.pricing.auto_sync);
        assert_eq!(cfg.cache.l2_hits, 3);
    }

    #[test]
    fn merge_into_empty_writes_all_defaults() {
        let text = merge_config("", None).unwrap();
        assert_eq!(Config::from_toml_str(&text).unwrap(), Config::default());
    }

    #[test]
    fn merge_preserves_existing_values_and_adds_new_keys() {
        let existing = "[modes]\nroute = \"enforce\"\n\n[cache]\nl2_hits = 7\n";
        let text = merge_config(existing, None).unwrap();
        let cfg = Config::from_toml_str(&text).unwrap();
        assert_eq!(cfg.modes.route, Mode::Enforce);
        assert_eq!(cfg.cache.l2_hits, 7);
        assert_eq!(cfg.modes.guard_read, Mode::Enforce);
        assert_eq!(cfg.cache.l3_hits, 10);
    }

    #[test]
    fn merge_is_idempotent_and_keeps_comments_when_complete() {
        let first = merge_config("", Some(Path::new("/c"))).unwrap();
        let commented = format!("# my comment\n{first}");
        let second = merge_config(&commented, Some(Path::new("/c"))).unwrap();
        assert_eq!(second, commented);
    }

    #[test]
    fn merge_records_claude_dir() {
        let text = merge_config("", Some(Path::new("/home/u/.claude"))).unwrap();
        let cfg = Config::from_toml_str(&text).unwrap();
        assert_eq!(cfg.claude.dir.as_deref(), Some("/home/u/.claude"));
    }

    #[test]
    fn merge_rejects_wrong_section_type() {
        let err = merge_config("modes = 3\n", None).unwrap_err();
        assert_eq!(err.to_string(), "`modes` in tokred.toml must be a table");
    }

    #[test]
    fn a_claude_key_that_is_not_a_table_is_rejected_when_saving_the_dir() {
        let err = merge_config("claude = 3\n", Some(Path::new("/c"))).unwrap_err();
        assert_eq!(err.to_string(), "`claude` in tokred.toml must be a table");
        // Without a dir to save, the same file is caught by the defaults merge.
        assert!(merge_config("claude = 3\n", None).is_err());
    }

    #[test]
    fn changing_the_saved_dir_rewrites_it() {
        let first = merge_config("", Some(Path::new("/a"))).unwrap();
        let second = merge_config(&first, Some(Path::new("/b"))).unwrap();
        assert_eq!(
            Config::from_toml_str(&second)
                .unwrap()
                .claude
                .dir
                .as_deref(),
            Some("/b")
        );
    }

    #[test]
    fn invalid_toml_is_an_error() {
        let err = merge_config("[[[", None).unwrap_err();
        assert!(err.to_string().starts_with("invalid TOML in tokred.toml"));
    }
}
