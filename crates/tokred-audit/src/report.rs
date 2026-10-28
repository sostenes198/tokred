//! The audit report and its machine-readable (`--json`) shape.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

/// Kind of fixed-context item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// A `CLAUDE.md` / `CLAUDE.local.md` file or a file imported from one.
    ClaudeMd,
    /// An always-loaded rule (`.claude/rules/**/*.md` without `paths` frontmatter).
    Rule,
    /// An installed skill (`SKILL.md`).
    Skill,
    /// An installed subagent definition.
    Agent,
}

impl ItemKind {
    /// Every kind, in report order.
    pub const ALL: [ItemKind; 4] = [Self::ClaudeMd, Self::Rule, Self::Skill, Self::Agent];
}

/// Where an item lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// The Claude Code folder (`~/.claude`).
    User,
    /// The project being audited.
    Project,
}

/// One weighed item of the fixed context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditItem {
    /// What the item is.
    pub kind: ItemKind,
    /// User or project level.
    pub scope: Scope,
    /// Display name (file name, skill/agent name, or the `@` import as written).
    pub name: String,
    /// Absolute path of the file.
    pub path: PathBuf,
    /// The file that imported this one, for `@path` imports.
    pub imported_by: Option<PathBuf>,
    /// Estimated tokens of the whole file (characters / 4, an estimate).
    pub estimated_tokens: u64,
    /// Estimated tokens that are in context at the start of every session, when known.
    ///
    /// Equal to `estimated_tokens` for memory files and rules; for skills only the listing entry
    /// (description and `when_to_use`, capped at 1536 characters) is always loaded; unknown for agents.
    pub startup_tokens: Option<u64>,
    /// Short suggested action, only when a documented threshold or problem applies.
    pub action: Option<String>,
}

/// Where an MCP server is configured. Declaration order is precedence order (highest first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpScope {
    /// `projects["<path>"].mcpServers` in `~/.claude.json` (private to one project).
    Local,
    /// `<project>/.mcp.json` (shared with the team).
    Project,
    /// Top-level `mcpServers` in `~/.claude.json` (all projects).
    User,
}

/// A configured MCP server compared with its real usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpServerReport {
    /// Server name as configured.
    pub name: String,
    /// Scope whose definition wins (local, then project, then user).
    pub scope: McpScope,
    /// Other scopes that also define a server with this name.
    pub also_in: Vec<McpScope>,
    /// File holding the winning definition.
    pub source: PathBuf,
    /// Transport (`type`; `stdio` when absent, as Claude Code reads it).
    pub transport: String,
    /// Whether a settings file lists the server in `disabledMcpjsonServers`.
    pub disabled: bool,
    /// `tool_use` calls inside the window.
    pub calls: u64,
    /// Unix seconds of the latest call inside the window.
    pub last_seen_unix: Option<i64>,
    /// True when usage is known and the server had no calls in the window.
    pub unused: bool,
    /// Short suggested action.
    pub action: Option<String>,
}

/// Usage of an MCP server that no local configuration file defines
/// (plugin-provided servers, claude.ai connectors, or servers since removed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnconfiguredUsage {
    /// Server segment of the tool name, as found in the transcripts.
    pub server: String,
    /// `tool_use` calls inside the window.
    pub calls: u64,
    /// Unix seconds of the latest call inside the window.
    pub last_seen_unix: Option<i64>,
}

/// Count and weight of one kind of item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct KindTotal {
    /// Number of items.
    pub count: usize,
    /// Sum of their estimated tokens.
    pub estimated_tokens: u64,
}

/// Totals of the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// Totals per item kind (every kind is always present).
    pub by_kind: BTreeMap<ItemKind, KindTotal>,
    /// Sum of the estimated tokens of all items.
    pub total_estimated_tokens: u64,
    /// Sum of the known start-of-session tokens of all items.
    pub total_startup_tokens: u64,
    /// Number of configured MCP servers.
    pub mcp_servers: usize,
    /// Number of configured MCP servers with no calls in the window.
    pub mcp_servers_unused: usize,
}

/// The result of `tokred audit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditReport {
    /// How token counts are computed (always an estimate).
    pub token_estimate: String,
    /// Length of the MCP usage window, in days.
    pub window_days: u32,
    /// Items ordered by weight, heaviest first.
    pub items: Vec<AuditItem>,
    /// Configured MCP servers: unused first, then by ascending calls, then by name.
    pub mcp_servers: Vec<McpServerReport>,
    /// Servers seen in the transcripts that no local config file defines.
    pub mcp_usage_without_local_config: Vec<UnconfiguredUsage>,
    /// Totals.
    pub summary: Summary,
    /// Limits of the report that the reader must know about.
    pub notes: Vec<String>,
    /// Problems met while auditing (unreadable files, malformed JSON, broken imports...).
    pub warnings: Vec<String>,
}
