//! `tokred configure`.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use tokred_core::{FileSystem, ManifestRepo};
use tokred_install::agents::AgentAction;
use tokred_install::settings::{ChangeKind, HookEntry, hook_entries};
use tokred_install::{
    ConfigureInput, ConfigurePlan, StdFileSystem, apply_configure, plan_configure,
};
use tokred_store::Store;

use super::{SystemClock, ask, existing_store, interactive, select_claude_dir, show};
use crate::binary;
use crate::env::Env;
use crate::logging;

pub fn run(env: &Env, dir_flag: Option<PathBuf>, yes: bool, force: bool) -> Result<()> {
    let fs = StdFileSystem;
    let claude_dir = select_claude_dir(&fs, env, dir_flag, true)?;
    let binary = binary::find()?;
    let version = env!("CARGO_PKG_VERSION");

    let store = existing_store(env)?;
    let input = ConfigureInput {
        claude_dir: &claude_dir,
        binary: &binary.path,
        version,
        force,
    };
    let manifest = store.as_ref().map(|s| s as &dyn ManifestRepo);
    let plan = plan_configure(&fs, manifest, &input)?;

    let rtk = binary::rtk_on_path()
        || plan
            .original_settings
            .as_deref()
            .is_some_and(|text| text.contains("rtk hook"));
    print_plan(&claude_dir, &binary, rtk, &plan);

    if !yes {
        if !interactive() {
            bail!("not running in a terminal; pass --yes to apply the plan above");
        }
        let answer = ask("Apply these changes? [y/N] ")?.unwrap_or_default();
        if !matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("Nothing was changed.");
            return Ok(());
        }
    }

    fs.create_dir_all(env.data.root())?;
    logging::init(&env.data.logs_dir());
    let config_path = env.data.config_path();
    let existing = fs.read_to_string(&config_path)?.unwrap_or_default();
    let merged = tokred_core::config::merge_config(&existing, Some(&claude_dir))
        .with_context(|| format!("could not update {}", config_path.display()))?;
    if merged != existing {
        fs.write_atomic(&config_path, merged.as_bytes())?;
    }

    let store = if let Some(store) = store {
        store
    } else {
        let db = env.data.db_path();
        Store::open(&db).with_context(|| format!("could not create {}", db.display()))?
    };
    // The database may have been created just now, so recompute against it.
    let plan = plan_configure(&fs, Some(&store as &dyn ManifestRepo), &input)?;
    let report = apply_configure(&fs, &store, &SystemClock, &plan)?;
    tracing::info!(claude_dir = %claude_dir.display(), "configured");

    println!();
    if report.settings_written {
        println!("Updated {}", show(&plan.settings_path));
        if report.backup_written {
            println!("Backup of the previous file: {}", show(&plan.backup_path));
        }
    } else {
        println!("settings.json already up to date.");
    }
    let conflicts: Vec<_> = report
        .agents
        .iter()
        .filter(|(_, a)| *a == AgentAction::Conflict)
        .map(|(f, _)| *f)
        .collect();
    for file in &conflicts {
        println!("You edited {file}: kept your version and wrote {file}.new next to it.");
    }
    println!("\nNext steps:\n  tokred doctor    check the installation");
    Ok(())
}

fn print_plan(
    claude_dir: &std::path::Path,
    binary: &binary::BinaryPath,
    rtk: bool,
    plan: &ConfigurePlan,
) {
    println!("tokred configure");
    println!("  Claude Code folder: {}", show(claude_dir));
    if binary.stable {
        println!("  Binary: {}", binary.path);
    } else {
        println!(
            "  Binary: {} (not found on PATH; hooks will point to this exact file)",
            binary.path
        );
    }
    if rtk {
        println!("  rtk detected: tokred and rtk will coexist (rtk keeps the Bash matcher)");
    }

    println!("\nHooks in {}:", show(&plan.settings_path));
    let desired: Vec<HookEntry> = hook_entries(&binary.path);
    for change in &plan.hook_changes {
        let symbol = match change.kind {
            ChangeKind::Added => '+',
            ChangeKind::Updated => '~',
            ChangeKind::Unchanged => '=',
            ChangeKind::Removed => '-',
        };
        let matcher = desired
            .iter()
            .find(|d| d.hook_arg() == Some(change.hook.as_str()))
            .and_then(|d| d.matcher.as_deref())
            .map(|m| format!(" [{m}]"))
            .unwrap_or_default();
        println!("  {symbol} {:<14} {}{matcher}", change.hook, change.event);
    }

    println!("\nAgents in agents/:");
    for agent in &plan.agents {
        let text = match agent.action {
            AgentAction::Create => "+ create",
            AgentAction::Update => "~ update",
            AgentAction::Unchanged => "= up to date",
            AgentAction::Conflict => "! you edited it: will write a .new file next to it",
            AgentAction::Overwrite => "~ overwrite (--force)",
        };
        println!("  {:<24} {text}", agent.file_name);
    }
    if plan.new_settings.is_some() && plan.original_settings.is_some() {
        println!("\nA backup will be saved as {}", show(&plan.backup_path));
    }
    println!();
}
