//! `tokred doctor`.

use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::Result;
use comfy_table::{Table, presets};
use serde_json::json;
use tokred_core::{Config, FileSystem, ManifestRepo};
use tokred_install::StdFileSystem;
use tokred_install::doctor::{AgentState, DoctorReport, HookState, doctor};

use super::{existing_store, select_claude_dir, show};
use crate::binary::{self, BinaryPath};
use crate::env::Env;
use crate::io::Io;

/// Facts about the machine that are not part of the Claude Code installation report.
struct Machine<'a> {
    env: &'a Env,
    binary: &'a BinaryPath,
    version: &'a str,
    schema_version: Option<u32>,
    rtk_binary: bool,
    config_exists: bool,
}

pub fn run(env: &Env, io: &mut Io, dir_flag: Option<PathBuf>, json: bool) -> Result<()> {
    let fs = StdFileSystem;
    let claude_dir = select_claude_dir(&fs, env, io, dir_flag, false)?;
    let binary = binary::find()?;
    let version = env!("CARGO_PKG_VERSION");

    let store = existing_store(env)?;
    let manifest = store.as_ref().map(|s| s as &dyn ManifestRepo);
    let mut report = doctor(&fs, manifest, &claude_dir, &binary.path, version)?;

    let config_path = env.data.config_path();
    let config_text = fs.read_to_string(&config_path)?;
    if let Some(Err(err)) = config_text.as_deref().map(Config::from_toml_str) {
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
    let machine = Machine {
        env,
        binary: &binary,
        version,
        schema_version: store.as_ref().and_then(|s| s.schema_version().ok()),
        rtk_binary: binary::rtk_on_path(),
        config_exists: config_text.is_some(),
    };

    if json {
        let value = json!({
            "tokred_version": machine.version,
            "binary": machine.binary.path,
            "binary_on_path": machine.binary.stable,
            "data_dir": machine.env.data.root(),
            "database": store.is_some(),
            "schema_version": machine.schema_version,
            "config": machine.config_exists,
            "rtk_binary_on_path": machine.rtk_binary,
            "install": report,
        });
        writeln!(io.out(), "{}", serde_json::to_string_pretty(&value)?)?;
    } else {
        let shown = render_human(&machine, &report);
        write!(io.out(), "{shown}")?;
    }
    Ok(())
}

/// The human-readable report. (Writing to a `String` cannot fail.)
fn render_human(machine: &Machine<'_>, report: &DoctorReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "tokred {}", machine.version);
    let _ = writeln!(out, "  binary:      {}", machine.binary.path);
    let _ = writeln!(out, "  claude dir:  {}", show(&report.claude_dir));
    let _ = writeln!(out, "  data dir:    {}", show(machine.env.data.root()));
    let _ = writeln!(
        out,
        "  database:    {}",
        machine
            .schema_version
            .map_or_else(|| "missing".to_owned(), |v| format!("ok (schema v{v})"))
    );
    let _ = writeln!(
        out,
        "  config:      {}",
        if machine.config_exists {
            "ok"
        } else {
            "missing"
        }
    );
    let _ = writeln!(
        out,
        "  rtk:         {}",
        if report.rtk_hooks_detected || machine.rtk_binary {
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
    let _ = writeln!(out, "\n{hooks}");

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
    let _ = writeln!(out, "\n{agents}");

    if report.issues.is_empty() {
        let _ = writeln!(out, "\nNo problems found.");
    } else {
        let _ = writeln!(out, "\nProblems:");
        for issue in &report.issues {
            let _ = writeln!(out, "  - {issue}");
        }
    }
    for note in &report.notes {
        let _ = writeln!(out, "  note: {note}");
    }
    out
}

#[cfg(test)]
mod tests {
    use tokred_install::doctor::{AgentStatus, HookStatus};

    use super::super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted;

    fn run_doctor(sb: &Sandbox, json: bool) -> (Result<()>, String) {
        let (mut io, out) = scripted(false, "");
        let result = run(&sb.env, &mut io, Some(sb.claude.clone()), json);
        (result, out.text())
    }

    fn configure(sb: &Sandbox) {
        let (mut io, _) = scripted(false, "");
        super::super::configure::run(&sb.env, &mut io, Some(sb.claude.clone()), true, false)
            .unwrap();
    }

    #[test]
    fn before_configure_it_lists_what_is_missing() {
        let sb = Sandbox::new();
        let (result, out) = run_doctor(&sb, false);
        result.unwrap();
        assert!(out.contains("database:    missing"), "{out}");
        assert!(out.contains("config:      missing"));
        assert!(out.contains("Problems:"));
        assert!(out.contains("database not found; run `tokred configure`"));
        assert!(out.contains("hook `guard-read` is not installed"));
        assert!(out.contains("tokred was not found on PATH"));
    }

    #[test]
    fn after_configure_everything_is_fine() {
        let sb = Sandbox::new();
        configure(&sb);
        let (result, out) = run_doctor(&sb, false);
        result.unwrap();
        assert!(out.contains("database:    ok (schema v1)"), "{out}");
        assert!(out.contains("config:      ok"));
        assert!(out.contains("No problems found."));
        assert!(out.contains("tokred-architect.md"));
        assert!(out.contains("current"));
    }

    #[test]
    fn an_invalid_config_is_a_problem() {
        let sb = Sandbox::new();
        configure(&sb);
        std::fs::write(sb.env.data.config_path(), "modes = 3\n").unwrap();
        let (result, out) = run_doctor(&sb, false);
        result.unwrap();
        assert!(out.contains("tokred.toml is invalid"), "{out}");
    }

    #[test]
    fn json_output_is_one_object() {
        let sb = Sandbox::new();
        configure(&sb);
        let (result, out) = run_doctor(&sb, true);
        result.unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(value["database"], true);
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["config"], true);
        assert_eq!(value["install"]["hooks"].as_array().unwrap().len(), 6);
    }

    #[test]
    fn human_output_renders_every_state() {
        let sb = Sandbox::new();
        let binary = BinaryPath {
            path: "/opt/bin/tokred".to_owned(),
            stable: true,
        };
        let hook = |name: &str, state, binary_missing| HookStatus {
            hook: name.to_owned(),
            event: "PreToolUse".to_owned(),
            state,
            binary_missing,
        };
        let agent = |file: &str, state, pending_new| AgentStatus {
            file: file.to_owned(),
            state,
            pending_new,
        };
        let report = DoctorReport {
            claude_dir: sb.claude.clone(),
            settings_exists: true,
            settings_valid: true,
            hooks: vec![
                hook("a", HookState::Ok, false),
                hook("b", HookState::Ok, true),
                hook("c", HookState::Missing, false),
                hook("d", HookState::Outdated, false),
            ],
            agents: vec![
                agent("w.md", AgentState::Current, false),
                agent("x.md", AgentState::Outdated, false),
                agent("y.md", AgentState::Modified, true),
                agent("z.md", AgentState::Missing, false),
            ],
            rtk_hooks_detected: false,
            issues: vec!["something is wrong".to_owned()],
            notes: vec!["something to know".to_owned()],
        };
        let machine = Machine {
            env: &sb.env,
            binary: &binary,
            version: "9.9.9",
            schema_version: None,
            rtk_binary: false,
            config_exists: false,
        };

        let text = render_human(&machine, &report);
        for expected in [
            "tokred 9.9.9",
            "/opt/bin/tokred",
            "database:    missing",
            "config:      missing",
            "rtk:         not detected",
            "ok, but binary path is gone",
            "missing",
            "outdated",
            "current",
            "edited by you (.new file pending)",
            "Problems:",
            "  - something is wrong",
            "  note: something to know",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
    }

    #[test]
    fn rtk_is_reported_when_its_hooks_are_present() {
        let sb = Sandbox::new();
        let binary = BinaryPath {
            path: "t".to_owned(),
            stable: true,
        };
        let report = DoctorReport {
            claude_dir: sb.claude.clone(),
            settings_exists: true,
            settings_valid: true,
            hooks: vec![],
            agents: vec![],
            rtk_hooks_detected: true,
            issues: vec![],
            notes: vec![],
        };
        let machine = Machine {
            env: &sb.env,
            binary: &binary,
            version: "1",
            schema_version: Some(1),
            rtk_binary: false,
            config_exists: true,
        };
        assert!(render_human(&machine, &report).contains("rtk:         detected"));
    }
}
