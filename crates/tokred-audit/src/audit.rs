//! The audit use case: weighs the fixed context and compares MCP servers with real usage.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;
use tokred_core::{Clock, FileSystem, Timestamp};

use crate::frontmatter::{self, Frontmatter};
use crate::mcp::{self, ServerDef};
use crate::memory::{
    candidate_specs, estimate_tokens, find_imports, looks_like_path, normalize, resolve_import,
    strip_block_comments,
};
use crate::ports::{DirEntry, DirLister, McpUsageSource, UsageScan};
use crate::report::{AuditItem, AuditReport, ItemKind, KindTotal, McpScope, Scope, Summary};

/// Default length of the MCP usage window, in days.
pub const DEFAULT_WINDOW_DAYS: u32 = 30;

/// Maximum number of `@` import hops Claude Code follows (documented).
const MAX_IMPORT_DEPTH: usize = 4;
/// Documented target for a `CLAUDE.md` file.
const CLAUDE_MD_MAX_LINES: usize = 200;
/// Claude Code skips memory files larger than this (documented).
const CLAUDE_MD_MAX_BYTES: usize = 4 * 1024 * 1024;
/// Documented recommendation for `SKILL.md`.
const SKILL_MAX_LINES: usize = 500;
/// The skill listing truncates `description` + `when_to_use` at this many characters (documented).
const SKILL_LISTING_MAX_CHARS: usize = 1536;
/// tokred heuristic (not documented): an agent prompt above this many tokens is worth trimming.
const AGENT_MAX_TOKENS: u64 = 2_000;
/// How deep `agents/` and `rules/` are walked.
const MAX_DIR_DEPTH: usize = 8;
const SECONDS_PER_DAY: i64 = 86_400;

const TOKEN_ESTIMATE: &str = "estimate: characters / 4, rounded up (not a real tokenizer count)";
const NOTE_MCP_WEIGHT: &str = "The weight of MCP tool schemas cannot be computed locally; MCP servers are compared by usage only.";
const NOTE_MCP_SOURCES: &str = "MCP servers provided by plugins, claude.ai connectors or managed settings are not defined in local config files; their usage is listed separately when seen in transcripts.";
const NOTE_SKILLS: &str = "Skills and agents are weighed by their whole file; only a skill's listing entry is in context before it is invoked (see startup_tokens).";

/// The outside world the audit reads from.
#[derive(Clone, Copy)]
pub struct Ports<'a> {
    /// File reads.
    pub fs: &'a dyn FileSystem,
    /// Directory listing.
    pub dirs: &'a dyn DirLister,
    /// MCP usage lookup.
    pub usage: &'a dyn McpUsageSource,
    /// Current time.
    pub clock: &'a dyn Clock,
}

/// What to audit.
#[derive(Debug, Clone)]
pub struct AuditRequest {
    /// The Claude Code folder (`~/.claude` or `$CLAUDE_CONFIG_DIR`).
    pub claude_dir: PathBuf,
    /// The user's home folder, used to expand `~` in imports.
    pub home_dir: Option<PathBuf>,
    /// Candidate locations of the `.claude.json` global config; the first that exists is used.
    pub global_config_candidates: Vec<PathBuf>,
    /// The project to audit, when auditing a project.
    pub project_dir: Option<PathBuf>,
    /// Length of the MCP usage window in days.
    pub window_days: u32,
}

/// Candidate locations of `.claude.json`: inside the Claude Code folder first (where a custom
/// `CLAUDE_CONFIG_DIR` keeps it), then the home folder (`~/.claude.json`, the documented default).
#[must_use]
pub fn global_config_candidates(claude_dir: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut out = vec![claude_dir.join(".claude.json")];
    out.extend(home.map(|h| h.join(".claude.json")));
    out
}

impl AuditRequest {
    /// A request with the default window and the default `.claude.json` candidates.
    #[must_use]
    pub fn new(
        claude_dir: impl Into<PathBuf>,
        home_dir: Option<PathBuf>,
        project_dir: Option<PathBuf>,
    ) -> Self {
        let claude_dir = claude_dir.into();
        Self {
            global_config_candidates: global_config_candidates(&claude_dir, home_dir.as_deref()),
            claude_dir,
            home_dir,
            project_dir,
            window_days: DEFAULT_WINDOW_DAYS,
        }
    }
}

/// Runs the audit. Problems with individual files become [`AuditReport::warnings`]; the audit
/// itself never fails.
#[must_use]
pub fn audit(ports: &Ports<'_>, request: &AuditRequest) -> AuditReport {
    let mut collector = Collector {
        ports,
        request,
        items: Vec::new(),
        warnings: Vec::new(),
        seen: HashSet::new(),
    };
    collector.collect_memory();
    collector.collect_rules();
    collector.collect_skills();
    collector.collect_agents();
    let (defs, usage) = collector.collect_mcp_defs();
    collector.finish(defs, usage.as_ref())
}

/// Result of reading one file.
enum Read {
    Text(String),
    Missing,
    Failed,
}

/// What happened when loading a memory file.
#[derive(PartialEq, Eq)]
enum Outcome {
    Loaded,
    Missing,
    Skipped,
}

#[derive(Clone, Copy)]
struct Origin {
    scope: Scope,
    kind: ItemKind,
}

struct Collector<'a> {
    ports: &'a Ports<'a>,
    request: &'a AuditRequest,
    items: Vec<AuditItem>,
    warnings: Vec<String>,
    seen: HashSet<PathBuf>,
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn is_hidden(path: &Path) -> bool {
    file_name(path).starts_with('.')
}

fn memory_action(text: &str) -> Option<String> {
    if text.len() > CLAUDE_MD_MAX_BYTES {
        Some("over 4 MiB: Claude Code skips this file; shrink or split it".to_owned())
    } else if text.trim().is_empty() {
        Some("empty file: consider removing".to_owned())
    } else if text.lines().count() > CLAUDE_MD_MAX_LINES {
        Some(format!(
            "over {CLAUDE_MD_MAX_LINES} lines: trim, or move topic-specific parts to path-scoped .claude/rules/ or a skill (@import does not reduce context cost)"
        ))
    } else {
        None
    }
}

/// Characters of a skill's listing entry (`description` plus `when_to_use`), before the cap.
fn listing_chars(fm: &Frontmatter) -> usize {
    ["description", "when_to_use"]
        .iter()
        .filter_map(|key| fm.fields.get(*key))
        .map(|value| value.chars().count())
        .sum()
}

fn defined_action(kind: ItemKind, text: &str, fm: &Frontmatter, tokens: u64) -> Option<String> {
    if fm.state == frontmatter::State::Unterminated {
        Some("fix the frontmatter: the closing `---` is missing".to_owned())
    } else if text.trim().is_empty() {
        Some("empty file: consider removing".to_owned())
    } else if kind == ItemKind::Skill && text.lines().count() > SKILL_MAX_LINES {
        Some(format!(
            "over {SKILL_MAX_LINES} lines: trim, or move reference material into supporting files"
        ))
    } else if kind == ItemKind::Skill && listing_chars(fm) > SKILL_LISTING_MAX_CHARS {
        Some(format!(
            "description + when_to_use exceed {SKILL_LISTING_MAX_CHARS} characters and are truncated in the skill listing"
        ))
    } else if kind == ItemKind::Agent && tokens > AGENT_MAX_TOKENS {
        Some("large system prompt: consider trimming".to_owned())
    } else {
        None
    }
}

impl Collector<'_> {
    fn warn(&mut self, message: String) {
        self.warnings.push(message);
    }

    fn read(&mut self, path: &Path) -> Read {
        match self.ports.fs.read_to_string(path) {
            Ok(Some(text)) => Read::Text(text),
            Ok(None) => Read::Missing,
            Err(err) => {
                self.warn(format!("could not read: {err}"));
                Read::Failed
            }
        }
    }

    fn list(&mut self, dir: &Path) -> Vec<DirEntry> {
        match self.ports.dirs.list_dir(dir) {
            Ok(entries) => entries,
            Err(err) => {
                self.warn(format!("could not list: {err}"));
                Vec::new()
            }
        }
    }

    /// Visible `.md` files under `dir`, recursively, in path order.
    fn md_files(&mut self, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        for entry in self.list(dir) {
            if is_hidden(&entry.path) {
                continue;
            }
            if entry.is_dir {
                if depth < MAX_DIR_DEPTH {
                    self.md_files(&entry.path, depth + 1, out);
                }
            } else if entry.path.extension().is_some_and(|ext| ext == "md") {
                out.push(entry.path);
            }
        }
    }

    /// Project directory and its ancestors: memory files are loaded from each of them.
    fn project_chain(&self) -> Vec<PathBuf> {
        self.request
            .project_dir
            .as_deref()
            .map(|dir| dir.ancestors().map(normalize).collect())
            .unwrap_or_default()
    }

    fn collect_memory(&mut self) {
        let user = Origin {
            scope: Scope::User,
            kind: ItemKind::ClaudeMd,
        };
        let project = Origin {
            scope: Scope::Project,
            kind: ItemKind::ClaudeMd,
        };
        let user_file = self.request.claude_dir.join("CLAUDE.md");
        self.load_memory(&user_file, user, None, None, 0, &mut Vec::new());
        for dir in self.project_chain() {
            for file in [
                dir.join("CLAUDE.md"),
                dir.join(".claude").join("CLAUDE.md"),
                dir.join("CLAUDE.local.md"),
            ] {
                self.load_memory(&file, project, None, None, 0, &mut Vec::new());
            }
        }
    }

    fn collect_rules(&mut self) {
        let mut roots = vec![(self.request.claude_dir.join("rules"), Scope::User)];
        if let Some(project) = &self.request.project_dir {
            roots.push((project.join(".claude").join("rules"), Scope::Project));
        }
        for (dir, scope) in roots {
            let mut files = Vec::new();
            self.md_files(&dir, 0, &mut files);
            let origin = Origin {
                scope,
                kind: ItemKind::Rule,
            };
            for file in files {
                self.load_memory(&file, origin, None, None, 0, &mut Vec::new());
            }
        }
    }

    /// Loads a memory file (or rule) and follows its `@path` imports.
    fn load_memory(
        &mut self,
        path: &Path,
        origin: Origin,
        name: Option<&str>,
        imported_by: Option<&Path>,
        depth: usize,
        stack: &mut Vec<PathBuf>,
    ) -> Outcome {
        let path = normalize(path);
        if stack.contains(&path) {
            self.warn(format!(
                "import cycle: `{}` imports itself (directly or through other files)",
                path.display()
            ));
            return Outcome::Skipped;
        }
        if !self.seen.insert(path.clone()) {
            return Outcome::Skipped;
        }
        let text = match self.read(&path) {
            Read::Text(text) => text,
            Read::Missing => return Outcome::Missing,
            Read::Failed => return Outcome::Skipped,
        };
        let path_scoped = origin.kind == ItemKind::Rule
            && depth == 0
            && frontmatter::parse(&text).fields.contains_key("paths");
        if path_scoped {
            return Outcome::Skipped;
        }
        let effective = strip_block_comments(&text);
        let tokens = estimate_tokens(&effective);
        self.items.push(AuditItem {
            kind: origin.kind,
            scope: origin.scope,
            name: name.map_or_else(|| file_name(&path), str::to_owned),
            path: path.clone(),
            imported_by: imported_by.map(Path::to_path_buf),
            estimated_tokens: tokens,
            startup_tokens: Some(tokens),
            action: memory_action(&effective),
        });
        stack.push(path.clone());
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        for spec in find_imports(&effective) {
            self.follow_import(&spec, &path, &base, origin, depth + 1, stack);
        }
        stack.pop();
        Outcome::Loaded
    }

    fn follow_import(
        &mut self,
        spec: &str,
        from: &Path,
        base: &Path,
        origin: Origin,
        depth: usize,
        stack: &mut Vec<PathBuf>,
    ) {
        let pathlike = looks_like_path(spec);
        if depth > MAX_IMPORT_DEPTH {
            if pathlike {
                self.warn(format!(
                    "import `@{spec}` in `{}` skipped: more than {MAX_IMPORT_DEPTH} hops deep",
                    from.display()
                ));
            }
            return;
        }
        let home = self.request.home_dir.clone();
        let mut outcome = Outcome::Missing;
        for candidate in candidate_specs(spec) {
            let Some(target) = resolve_import(&candidate, base, home.as_deref()) else {
                continue;
            };
            outcome = self.load_memory(&target, origin, Some(spec), Some(from), depth, stack);
            if outcome != Outcome::Missing {
                break;
            }
        }
        if outcome == Outcome::Missing && pathlike {
            self.warn(format!(
                "import `@{spec}` in `{}` not found",
                from.display()
            ));
        }
    }

    fn collect_skills(&mut self) {
        let mut roots = vec![(self.request.claude_dir.join("skills"), Scope::User)];
        if let Some(project) = &self.request.project_dir {
            roots.push((project.join(".claude").join("skills"), Scope::Project));
        }
        for (dir, scope) in roots {
            for entry in self.list(&dir) {
                if !entry.is_dir || is_hidden(&entry.path) {
                    continue;
                }
                let file = normalize(&entry.path.join("SKILL.md"));
                self.add_defined(&file, ItemKind::Skill, scope, &file_name(&entry.path));
            }
        }
    }

    fn collect_agents(&mut self) {
        let mut roots = vec![(self.request.claude_dir.join("agents"), Scope::User)];
        if let Some(project) = &self.request.project_dir {
            roots.push((project.join(".claude").join("agents"), Scope::Project));
        }
        for (dir, scope) in roots {
            let mut files = Vec::new();
            self.md_files(&dir, 0, &mut files);
            for file in files {
                let stem = file
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.add_defined(&normalize(&file), ItemKind::Agent, scope, &stem);
            }
        }
    }

    /// Adds a skill or an agent: the whole file is weighed, the name comes from the frontmatter.
    fn add_defined(&mut self, path: &Path, kind: ItemKind, scope: Scope, fallback: &str) {
        if !self.seen.insert(path.to_path_buf()) {
            return;
        }
        let Read::Text(text) = self.read(path) else {
            return;
        };
        let fm = frontmatter::parse(&text);
        if fm.state == frontmatter::State::Unterminated {
            self.warn(format!(
                "invalid frontmatter in `{}`: the closing `---` is missing",
                path.display()
            ));
        }
        let tokens = estimate_tokens(&text);
        let startup_tokens = (kind == ItemKind::Skill)
            .then(|| estimate_tokens(&"x".repeat(listing_chars(&fm).min(SKILL_LISTING_MAX_CHARS))));
        let name = fm
            .fields
            .get("name")
            .filter(|n| !n.is_empty())
            .map_or_else(|| fallback.to_owned(), Clone::clone);
        self.items.push(AuditItem {
            action: defined_action(kind, &text, &fm, tokens),
            path: path.to_path_buf(),
            imported_by: None,
            estimated_tokens: tokens,
            startup_tokens,
            kind,
            scope,
            name,
        });
    }

    fn read_json(&mut self, path: &Path) -> Option<Value> {
        let Read::Text(text) = self.read(path) else {
            return None;
        };
        self.parse_json(path, &text)
    }

    fn parse_json(&mut self, path: &Path, text: &str) -> Option<Value> {
        let parsed = serde_json::from_str::<Value>(text);
        if let Err(err) = &parsed {
            self.warn(format!("malformed JSON in `{}`: {err}", path.display()));
        }
        parsed.ok()
    }

    /// Reads every MCP configuration file; returns the definitions and the usage, when known.
    fn collect_mcp_defs(&mut self) -> (Vec<ServerDef>, Option<UsageScan>) {
        let mut defs = Vec::new();
        let mut add = |servers: Option<&Value>, scope: McpScope, source: &Path| {
            for (name, transport) in mcp::parse_server_map(servers) {
                defs.push(ServerDef {
                    name,
                    scope,
                    source: source.to_path_buf(),
                    transport,
                    disabled: false,
                });
            }
        };
        let candidates = self.request.global_config_candidates.clone();
        let mut global = None;
        for candidate in candidates {
            if let Read::Text(text) = self.read(&candidate) {
                global = self
                    .parse_json(&candidate, &text)
                    .map(|json| (candidate, json));
                break;
            }
        }
        if let Some((source, json)) = &global {
            add(json.get("mcpServers"), McpScope::User, source);
            if let Some(project) = &self.request.project_dir {
                let project = project.to_string_lossy();
                add(mcp::local_servers(json, &project), McpScope::Local, source);
            }
        }
        let mut disabled: Vec<String> = Vec::new();
        if let Some(project) = self.request.project_dir.clone() {
            let file = project.join(".mcp.json");
            if let Some(json) = self.read_json(&file) {
                add(json.get("mcpServers"), McpScope::Project, &file);
            }
            for settings in [
                self.request.claude_dir.join("settings.json"),
                project.join(".claude").join("settings.json"),
                project.join(".claude").join("settings.local.json"),
            ] {
                if let Some(json) = self.read_json(&settings) {
                    disabled.extend(mcp::disabled_names(&json));
                }
            }
        }
        for def in &mut defs {
            def.disabled = def.scope == McpScope::Project && disabled.contains(&def.name);
        }
        let scan = self.lookup_usage();
        (defs, scan)
    }

    /// Asks the usage source; `None` when usage cannot be judged (error or no transcripts).
    fn lookup_usage(&mut self) -> Option<UsageScan> {
        let now = self.ports.clock.now();
        let since = Timestamp(now.0 - i64::from(self.request.window_days) * SECONDS_PER_DAY);
        match self.ports.usage.usage_since(since) {
            Err(err) => {
                self.warn(format!("MCP usage is unknown: {err}"));
                None
            }
            Ok(scan) if scan.files_scanned == 0 => {
                self.warn(
                    "no transcripts modified inside the window were found: MCP usage cannot be judged"
                        .to_owned(),
                );
                None
            }
            Ok(scan) => Some(scan),
        }
    }

    fn finish(mut self, defs: Vec<ServerDef>, usage: Option<&UsageScan>) -> AuditReport {
        let window_days = self.request.window_days;
        let (mcp_servers, mcp_usage_without_local_config) = mcp::compare(defs, usage, window_days);
        self.items.sort_by(|a, b| {
            b.estimated_tokens
                .cmp(&a.estimated_tokens)
                .then(a.kind.cmp(&b.kind))
                .then(a.scope.cmp(&b.scope))
                .then_with(|| a.path.cmp(&b.path))
        });
        let mut by_kind: BTreeMap<ItemKind, KindTotal> = ItemKind::ALL
            .iter()
            .map(|kind| (*kind, KindTotal::default()))
            .collect();
        for item in &self.items {
            let total = by_kind.entry(item.kind).or_default();
            total.count += 1;
            total.estimated_tokens += item.estimated_tokens;
        }
        let summary = Summary {
            total_estimated_tokens: self.items.iter().map(|i| i.estimated_tokens).sum(),
            total_startup_tokens: self.items.iter().filter_map(|i| i.startup_tokens).sum(),
            mcp_servers: mcp_servers.len(),
            mcp_servers_unused: mcp_servers.iter().filter(|s| s.unused).count(),
            by_kind,
        };
        AuditReport {
            token_estimate: TOKEN_ESTIMATE.to_owned(),
            window_days,
            items: self.items,
            mcp_servers,
            mcp_usage_without_local_config,
            summary,
            notes: [NOTE_MCP_WEIGHT, NOTE_MCP_SOURCES, NOTE_SKILLS]
                .map(str::to_owned)
                .to_vec(),
            warnings: self.warnings,
        }
    }
}
