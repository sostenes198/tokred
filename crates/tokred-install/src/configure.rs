//! `configure`: plan what will change, then apply it.
//!
//! Planning only reads; applying writes. The CLI shows the plan and asks for confirmation in
//! between (spec section 5.0).

use std::path::{Path, PathBuf};

use serde_json::Value;
use tokred_core::{Clock, FileSystem, ManifestEntry, ManifestRepo};

use crate::agents::{
    AgentAction, TEMPLATES, agents_dir, conflict_path, content_hash, plan_agent, render,
};
use crate::error::InstallError;
use crate::settings::{HookChange, hook_entries, merge_settings, plan_hook_changes};

/// Name of the settings backup (only the last one is kept).
const BACKUP_SUFFIX: &str = ".tokred.bak";

/// Inputs of a configure run.
#[derive(Debug, Clone, Copy)]
pub struct ConfigureInput<'a> {
    /// Claude Code folder.
    pub claude_dir: &'a Path,
    /// Stable path of the tokred binary, as it will be written in the hook commands.
    pub binary: &'a str,
    /// tokred version (e.g. `0.1.0`).
    pub version: &'a str,
    /// Overwrite agent files even if the user edited them.
    pub force: bool,
}

/// What will happen to one agent file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPlan {
    /// File name inside `<claude>/agents/`.
    pub file_name: &'static str,
    /// Full path.
    pub path: PathBuf,
    /// Decision.
    pub action: AgentAction,
    /// Rendered content to install.
    pub content: String,
    /// blake3 hash of `content`.
    pub hash: String,
    /// The manifest row is missing or out of date and must be rewritten.
    pub manifest_stale: bool,
}

/// Everything `configure` is about to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurePlan {
    /// The Claude Code folder.
    pub claude_dir: PathBuf,
    /// `<claude>/agents/`.
    pub agents_dir: PathBuf,
    /// `<claude>/settings.json`.
    pub settings_path: PathBuf,
    /// `<claude>/settings.json.tokred.bak`.
    pub backup_path: PathBuf,
    /// Original settings text (`None` if the file did not exist); used for the backup.
    pub original_settings: Option<String>,
    /// New settings text; `None` when the file is already up to date.
    pub new_settings: Option<String>,
    /// Per-hook changes.
    pub hook_changes: Vec<HookChange>,
    /// Per-agent actions.
    pub agents: Vec<AgentPlan>,
    /// tokred version recorded in the manifest.
    pub version: String,
}

/// Outcome of [`apply_configure`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigureReport {
    /// `settings.json` was written.
    pub settings_written: bool,
    /// A backup of the previous `settings.json` was written.
    pub backup_written: bool,
    /// Action taken per agent file.
    pub agents: Vec<(&'static str, AgentAction)>,
}

pub(crate) fn settings_path(claude_dir: &Path) -> PathBuf {
    claude_dir.join("settings.json")
}

pub(crate) fn backup_path(claude_dir: &Path) -> PathBuf {
    claude_dir.join(format!("settings.json{BACKUP_SUFFIX}"))
}

/// Reads and parses `settings.json`. A missing or blank file is an empty object; invalid JSON
/// is an error so that nothing is ever written over a file we could not understand.
pub(crate) fn read_settings(
    fs: &dyn FileSystem,
    path: &Path,
) -> Result<(Value, Option<String>), InstallError> {
    let text = fs.read_to_string(path)?;
    let value = match text.as_deref() {
        None => Value::Object(serde_json::Map::new()),
        Some(raw) if raw.trim().is_empty() => Value::Object(serde_json::Map::new()),
        Some(raw) => serde_json::from_str(raw).map_err(|source| InstallError::InvalidJson {
            path: path.to_owned(),
            source,
        })?,
    };
    Ok((value, text))
}

pub(crate) fn serialize_settings(value: &Value) -> Result<String, InstallError> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    Ok(text)
}

/// Computes what `configure` would do, without changing anything.
pub fn plan_configure(
    fs: &dyn FileSystem,
    manifest: Option<&dyn ManifestRepo>,
    input: &ConfigureInput<'_>,
) -> Result<ConfigurePlan, InstallError> {
    let settings_path = settings_path(input.claude_dir);
    let (current, original_settings) = read_settings(fs, &settings_path)?;
    let desired = hook_entries(input.binary);
    let hook_changes = plan_hook_changes(&current, &desired);
    let merged =
        merge_settings(current.clone(), &desired).map_err(|source| InstallError::Merge {
            path: settings_path.clone(),
            source,
        })?;
    let new_settings = if original_settings.is_some() && merged == current {
        None
    } else {
        Some(serialize_settings(&merged)?)
    };

    let dir = agents_dir(input.claude_dir);
    let mut agent_plans = Vec::with_capacity(TEMPLATES.len());
    for template in TEMPLATES {
        let path = dir.join(template.file_name);
        let content = render(&template, input.version);
        let hash = content_hash(content.as_bytes());
        let current_hash = fs
            .read_to_string(&path)?
            .map(|text| content_hash(text.as_bytes()));
        let manifest_hash = match manifest {
            Some(repo) => repo.get(&path)?.map(|entry| entry.hash),
            None => None,
        };
        let action = plan_agent(
            &hash,
            current_hash.as_deref(),
            manifest_hash.as_deref(),
            input.force,
        );
        let manifest_stale =
            action == AgentAction::Unchanged && manifest_hash.as_deref() != Some(&hash);
        agent_plans.push(AgentPlan {
            file_name: template.file_name,
            path,
            action,
            content,
            hash,
            manifest_stale,
        });
    }

    Ok(ConfigurePlan {
        backup_path: backup_path(input.claude_dir),
        claude_dir: input.claude_dir.to_owned(),
        agents_dir: dir,
        settings_path,
        original_settings,
        new_settings,
        hook_changes,
        agents: agent_plans,
        version: input.version.to_owned(),
    })
}

/// Applies a plan produced by [`plan_configure`].
pub fn apply_configure(
    fs: &dyn FileSystem,
    manifest: &dyn ManifestRepo,
    clock: &dyn Clock,
    plan: &ConfigurePlan,
) -> Result<ConfigureReport, InstallError> {
    let mut settings_written = false;
    let mut backup_written = false;
    if let Some(new_settings) = &plan.new_settings {
        if let Some(original) = &plan.original_settings {
            fs.write_atomic(&plan.backup_path, original.as_bytes())?;
            backup_written = true;
        }
        fs.create_dir_all(&plan.claude_dir)?;
        fs.write_atomic(&plan.settings_path, new_settings.as_bytes())?;
        settings_written = true;
    }

    let mut done = Vec::with_capacity(plan.agents.len());
    fs.create_dir_all(&plan.agents_dir)?;
    for agent in &plan.agents {
        let stale_new = conflict_path(&agent.path);
        match agent.action {
            AgentAction::Create | AgentAction::Update | AgentAction::Overwrite => {
                fs.write_atomic(&agent.path, agent.content.as_bytes())?;
                fs.remove_file(&stale_new)?;
                record(manifest, clock, agent, &plan.version)?;
            }
            AgentAction::Unchanged => {
                fs.remove_file(&stale_new)?;
                if agent.manifest_stale {
                    record(manifest, clock, agent, &plan.version)?;
                }
            }
            AgentAction::Conflict => {
                fs.write_atomic(&stale_new, agent.content.as_bytes())?;
            }
        }
        done.push((agent.file_name, agent.action));
    }

    Ok(ConfigureReport {
        settings_written,
        backup_written,
        agents: done,
    })
}

fn record(
    manifest: &dyn ManifestRepo,
    clock: &dyn Clock,
    agent: &AgentPlan,
    version: &str,
) -> Result<(), InstallError> {
    manifest.upsert(&ManifestEntry {
        path: agent.path.clone(),
        hash: agent.hash.clone(),
        version: version.to_owned(),
        installed_at: clock.now(),
    })?;
    Ok(())
}
