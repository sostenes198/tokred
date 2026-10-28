//! `doctor`: inspects what is installed in the Claude Code folder.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tokred_core::{FileSystem, HookEvent, ManifestRepo};

use crate::agents::{TEMPLATES, agents_dir, conflict_path, content_hash, render};
use crate::configure::{read_settings, settings_path};
use crate::error::InstallError;
use crate::settings::{HookEntry, foreign_entries, hook_entries, tokred_entries};

/// State of one hook in `settings.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookState {
    /// Installed and identical to what this version would install.
    Ok,
    /// Not installed.
    Missing,
    /// Installed but different (other binary path, matcher or timeout).
    Outdated,
}

/// Status of one hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookStatus {
    /// Hook name (`guard-read`, ...).
    pub hook: String,
    /// Claude Code event.
    pub event: String,
    /// State.
    pub state: HookState,
    /// The binary path written in the command no longer exists.
    pub binary_missing: bool,
}

/// State of one agent file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Installed and up to date.
    Current,
    /// An untouched file from an older version.
    Outdated,
    /// The user edited the file.
    Modified,
    /// Not installed.
    Missing,
}

/// Status of one agent file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentStatus {
    /// File name.
    pub file: String,
    /// State.
    pub state: AgentState,
    /// A `.new` file is waiting next to it.
    pub pending_new: bool,
}

/// Result of [`doctor`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    /// Claude Code folder inspected.
    pub claude_dir: PathBuf,
    /// `settings.json` exists.
    pub settings_exists: bool,
    /// `settings.json` parses as JSON.
    pub settings_valid: bool,
    /// Per-hook status.
    pub hooks: Vec<HookStatus>,
    /// Per-agent status.
    pub agents: Vec<AgentStatus>,
    /// rtk hooks are present in `settings.json`.
    pub rtk_hooks_detected: bool,
    /// Problems that need attention.
    pub issues: Vec<String>,
    /// Informational notes.
    pub notes: Vec<String>,
}

/// Extracts the executable path from a hook command (quoted or not).
fn command_binary(command: &str) -> &str {
    match command.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest),
        None => command.split_whitespace().next().unwrap_or(command),
    }
}

/// Inspects the installation.
pub fn doctor(
    fs: &dyn FileSystem,
    manifest: Option<&dyn ManifestRepo>,
    claude_dir: &Path,
    binary: &str,
    version: &str,
) -> Result<DoctorReport, InstallError> {
    let mut issues = Vec::new();
    let mut notes = Vec::new();
    let settings = settings_path(claude_dir);
    let settings_exists = fs.exists(&settings);
    let (value, settings_valid) = match read_settings(fs, &settings) {
        Ok((value, _)) => (value, true),
        Err(InstallError::InvalidJson { .. }) => {
            issues.push("settings.json is not valid JSON; tokred will not touch it".to_owned());
            (serde_json::Value::Null, false)
        }
        Err(other) => return Err(other),
    };

    let hooks = check_hooks(fs, &value, binary);
    for hook in &hooks {
        match hook.state {
            HookState::Missing => issues.push(format!("hook `{}` is not installed", hook.hook)),
            HookState::Outdated => issues.push(format!("hook `{}` is outdated", hook.hook)),
            HookState::Ok => {}
        }
        if hook.binary_missing {
            issues.push(format!(
                "hook `{}` points to a binary that no longer exists",
                hook.hook
            ));
        }
    }
    if hooks
        .iter()
        .any(|h| h.state != HookState::Ok || h.binary_missing)
    {
        notes.push("run `tokred configure` to fix the hooks".to_owned());
    }

    let agents = check_agents(fs, manifest, claude_dir, version)?;
    for agent in &agents {
        match agent.state {
            AgentState::Missing => issues.push(format!("agent `{}` is not installed", agent.file)),
            AgentState::Outdated => issues.push(format!("agent `{}` is outdated", agent.file)),
            AgentState::Modified => notes.push(format!(
                "agent `{0}` was edited by you; the new version is in `{0}.new`",
                agent.file
            )),
            AgentState::Current => {}
        }
    }

    let rtk_hooks_detected = check_coexistence(&value, &mut issues, &mut notes);

    Ok(DoctorReport {
        claude_dir: claude_dir.to_owned(),
        settings_exists,
        settings_valid,
        hooks,
        agents,
        rtk_hooks_detected,
        issues,
        notes,
    })
}

fn check_hooks(fs: &dyn FileSystem, settings: &serde_json::Value, binary: &str) -> Vec<HookStatus> {
    let desired = hook_entries(binary);
    let installed = tokred_entries(settings);
    HookEvent::ALL
        .into_iter()
        .zip(&desired)
        .map(|(hook, want)| {
            let have: Option<&HookEntry> = installed
                .iter()
                .find(|entry| entry.hook_arg() == Some(hook.arg()));
            let state = match have {
                None => HookState::Missing,
                Some(have) if have == want => HookState::Ok,
                Some(_) => HookState::Outdated,
            };
            let binary_missing =
                have.is_some_and(|have| !fs.exists(Path::new(command_binary(&have.command))));
            HookStatus {
                hook: hook.arg().to_owned(),
                event: hook.claude_event().to_owned(),
                state,
                binary_missing,
            }
        })
        .collect()
}

fn check_agents(
    fs: &dyn FileSystem,
    manifest: Option<&dyn ManifestRepo>,
    claude_dir: &Path,
    version: &str,
) -> Result<Vec<AgentStatus>, InstallError> {
    let dir = agents_dir(claude_dir);
    let mut agents = Vec::with_capacity(TEMPLATES.len());
    for template in TEMPLATES {
        let path = dir.join(template.file_name);
        let desired_hash = content_hash(render(&template, version).as_bytes());
        let current = fs
            .read_to_string(&path)?
            .map(|text| content_hash(text.as_bytes()));
        let recorded = match manifest {
            Some(repo) => repo.get(&path)?.map(|entry| entry.hash),
            None => None,
        };
        let state = match current.as_deref() {
            None => AgentState::Missing,
            Some(hash) if hash == desired_hash => AgentState::Current,
            Some(hash) if recorded.as_deref() == Some(hash) => AgentState::Outdated,
            Some(_) => AgentState::Modified,
        };
        agents.push(AgentStatus {
            file: template.file_name.to_owned(),
            state,
            pending_new: fs.exists(&conflict_path(&path)),
        });
    }
    Ok(agents)
}

/// Looks for rtk and for overlaps with other hooks; returns whether rtk hooks are present.
fn check_coexistence(
    settings: &serde_json::Value,
    issues: &mut Vec<String>,
    notes: &mut Vec<String>,
) -> bool {
    let foreign = foreign_entries(settings);
    let rtk_hooks_detected = foreign
        .iter()
        .any(|entry| entry.command.contains("rtk hook"));
    if rtk_hooks_detected {
        notes.push(
            "rtk hooks detected: tokred and rtk coexist (rtk uses the Bash matcher)".to_owned(),
        );
    }
    for tokred in tokred_entries(settings) {
        let name = tokred.hook_arg().unwrap_or("?").to_owned();
        let Some(matcher) = tokred.matcher.as_deref() else {
            continue;
        };
        if matcher.split('|').any(|part| part == "Bash") {
            issues.push(format!(
                "tokred hook `{name}` matches Bash, which is reserved for rtk"
            ));
        }
        if foreign
            .iter()
            .any(|other| other.event == tokred.event && other.matcher.as_deref() == Some(matcher))
        {
            notes.push(format!(
                "another hook shares the `{matcher}` matcher of `{name}`"
            ));
        }
    }
    rtk_hooks_detected
}
