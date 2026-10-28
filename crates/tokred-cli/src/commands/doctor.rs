//! `tokred doctor`.

use std::path::PathBuf;

use anyhow::Result;
use comfy_table::{Table, presets};
use serde_json::json;
use tokred_core::{Config, FileSystem, ManifestRepo};
use tokred_install::StdFileSystem;
use tokred_install::doctor::{AgentState, DoctorReport, HookState, doctor};

use super::{existing_store, select_claude_dir, show};
use crate::binary;
use crate::env::Env;

pub fn run(env: &Env, dir_flag: Option<PathBuf>, json: bool) -> Result<()> {
    let fs = StdFileSystem;
    let claude_dir = select_claude_dir(&fs, env, dir_flag, false)?;
    let binary = binary::find()?;
    let version = env!("CARGO_PKG_VERSION");

    let store = existing_store(env)?;
    let manifest = store.as_ref().map(|s| s as &dyn ManifestRepo);
    let mut report = doctor(&fs, manifest, &claude_dir, &binary.path, version)?;

    let config_path = env.data.config_path();
    let config_text = fs.read_to_string(&config_path)?;
    let config_ok = config_text.as_deref().map(Config::from_toml_str);
    if let Some(Err(err)) = &config_ok {
        report
            .issues
            .push(format!("{} is invalid: {err}", show(&config_path)));
    }
    if store.is_none() {
        report
            .issues
            .push("database not found; run `tokred configure`".to_owned());
    }
    if !binary.stable {
        report.notes.push(
            "tokred was not found on PATH; hooks use the exact executable path and will break \
             if it moves"
                .to_owned(),
        );
    }
    let schema_version = store.as_ref().and_then(|s| s.schema_version().ok());
    let rtk_binary = binary::rtk_on_path();

    if json {
        let value = json!({
            "tokred_version": version,
            "binary": binary.path,
            "binary_on_path": binary.stable,
            "data_dir": env.data.root(),
            "database": store.is_some(),
            "schema_version": schema_version,
            "config": config_text.is_some(),
            "rtk_binary_on_path": rtk_binary,
            "install": report,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        print_human(
            env,
            &binary,
            version,
            schema_version,
            rtk_binary,
            &report,
            config_text.is_some(),
        );
    }
    Ok(())
}

fn print_human(
    env: &Env,
    binary: &binary::BinaryPath,
    version: &str,
    schema_version: Option<u32>,
    rtk_binary: bool,
    report: &DoctorReport,
    config_exists: bool,
) {
    println!("tokred {version}");
    println!("  binary:      {}", binary.path);
    println!("  claude dir:  {}", show(&report.claude_dir));
    println!("  data dir:    {}", show(env.data.root()));
    println!(
        "  database:    {}",
        schema_version.map_or_else(|| "missing".to_owned(), |v| format!("ok (schema v{v})"))
    );
    println!(
        "  config:      {}",
        if config_exists { "ok" } else { "missing" }
    );
    println!(
        "  rtk:         {}",
        if report.rtk_hooks_detected || rtk_binary {
            "detected"
        } else {
            "not detected"
        }
    );

    let mut hooks = Table::new();
    hooks.load_style(presets::ASCII_BORDERS_ONLY_CONDENSED);
    hooks.set_header(["Hook", "Event", "State"]);
    for hook in &report.hooks {
        let state = match hook.state {
            HookState::Ok if hook.binary_missing => "ok, but binary path is gone",
            HookState::Ok => "ok",
            HookState::Missing => "missing",
            HookState::Outdated => "outdated",
        };
        hooks.add_row([hook.hook.as_str(), hook.event.as_str(), state]);
    }
    println!("\n{hooks}");

    let mut agents = Table::new();
    agents.load_style(presets::ASCII_BORDERS_ONLY_CONDENSED);
    agents.set_header(["Agent", "State"]);
    for agent in &report.agents {
        let mut state = match agent.state {
            AgentState::Current => "current",
            AgentState::Outdated => "outdated",
            AgentState::Modified => "edited by you",
            AgentState::Missing => "missing",
        }
        .to_owned();
        if agent.pending_new {
            state.push_str(" (.new file pending)");
        }
        agents.add_row([agent.file.as_str(), state.as_str()]);
    }
    println!("\n{agents}");

    if report.issues.is_empty() {
        println!("\nNo problems found.");
    } else {
        println!("\nProblems:");
        for issue in &report.issues {
            println!("  - {issue}");
        }
    }
    for note in &report.notes {
        println!("  note: {note}");
    }
}
