//! End-to-end tests: run the real `tokred` binary inside a fully sandboxed environment.
//!
//! `HOME`, `CLAUDE_CONFIG_DIR` and `TOKRED_DATA_DIR` all point into a temporary directory, so
//! nothing here can ever touch the real `~/.claude` or the real tokred data folder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use serde_json::Value;

struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        // The Claude folder lives in a directory with a space, like many Windows paths.
        std::fs::create_dir_all(root.path().join("my claude/projects")).unwrap();
        std::fs::create_dir_all(root.path().join("home")).unwrap();
        Self { root }
    }

    fn claude(&self) -> PathBuf {
        self.root.path().join("my claude")
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("tokred data")
    }

    fn settings(&self) -> PathBuf {
        self.claude().join("settings.json")
    }

    fn write_settings(&self, text: &str) {
        std::fs::write(self.settings(), text).unwrap();
    }

    fn settings_json(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.settings()).unwrap()).unwrap()
    }

    fn tokred(&self) -> Command {
        let home = self.root.path().join("home");
        let mut cmd = cargo_bin_cmd!("tokred");
        cmd.env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", home.join("appdata"))
            .env("XDG_DATA_HOME", home.join("xdg"))
            .env("CLAUDE_CONFIG_DIR", self.claude())
            .env("TOKRED_DATA_DIR", self.data())
            .env_remove("TOKRED_LOG");
        cmd
    }
}

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn tokred_hook_count(settings: &Value) -> usize {
    let mut count = 0;
    for groups in settings["hooks"]
        .as_object()
        .into_iter()
        .flat_map(|h| h.values())
    {
        for group in groups.as_array().unwrap() {
            for handler in group["hooks"].as_array().unwrap() {
                if handler["command"].as_str().unwrap().contains(" hook ")
                    && handler["command"].as_str().unwrap().contains("tokred")
                {
                    count += 1;
                }
            }
        }
    }
    count
}

/// The quickstart from the README: configure, doctor, unconfigure.
#[test]
fn quickstart_configure_doctor_unconfigure() {
    let sb = Sandbox::new();
    let original = fixture("settings-rtk.json");
    sb.write_settings(&original);

    sb.tokred()
        .args(["configure", "--yes"])
        .assert()
        .success()
        .stderr("")
        .stdout(predicate::str::contains("Hooks in"))
        .stdout(predicate::str::contains("tokred-dev.md"))
        .stdout(predicate::str::contains("rtk detected"));

    assert_eq!(tokred_hook_count(&sb.settings_json()), 6);
    assert!(sb.data().join("tokred.db").exists());
    assert!(sb.data().join("tokred.toml").exists());
    let logs: Vec<_> = std::fs::read_dir(sb.data().join("logs")).unwrap().collect();
    assert!(!logs.is_empty(), "configure writes a log file");
    assert!(sb.claude().join("agents/tokred-explorer.md").exists());
    assert_eq!(
        std::fs::read_to_string(sb.claude().join("settings.json.tokred.bak")).unwrap(),
        original
    );

    // The chosen folder is remembered.
    let toml = std::fs::read_to_string(sb.data().join("tokred.toml")).unwrap();
    assert!(
        toml.contains("[claude]") && toml.contains("my claude"),
        "{toml}"
    );

    // Running it again changes nothing.
    let before = std::fs::read_to_string(sb.settings()).unwrap();
    sb.tokred().args(["configure", "--yes"]).assert().success();
    assert_eq!(std::fs::read_to_string(sb.settings()).unwrap(), before);

    // doctor, human and JSON.
    sb.tokred()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("guard-read"))
        .stdout(predicate::str::contains("tokred-architect.md"));
    let out = sb
        .tokred()
        .args(["doctor", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["database"], true);
    assert_eq!(json["schema_version"], 1);
    insta::assert_json_snapshot!(json["install"]["hooks"]);

    // unconfigure restores the original (semantically) and removes the agents.
    sb.tokred().arg("unconfigure").assert().success();
    let expected: Value = serde_json::from_str(&original).unwrap();
    assert_eq!(sb.settings_json(), expected);
    assert!(!sb.claude().join("agents/tokred-dev.md").exists());
    assert!(sb.data().exists(), "data folder stays without --purge");

    sb.tokred()
        .args(["unconfigure", "--purge"])
        .assert()
        .success();
    assert!(!sb.data().exists());
}

#[test]
fn invalid_settings_json_aborts_and_changes_nothing() {
    let sb = Sandbox::new();
    let broken = fixture("settings-invalid.json");
    sb.write_settings(&broken);

    sb.tokred()
        .args(["configure", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not valid JSON"));
    assert_eq!(std::fs::read_to_string(sb.settings()).unwrap(), broken);
    assert!(!sb.claude().join("settings.json.tokred.bak").exists());
    assert!(!sb.claude().join("agents").exists());
    assert!(
        !sb.data().exists(),
        "nothing is created before the plan is accepted"
    );
}

#[test]
fn configure_needs_confirmation_outside_a_terminal() {
    let sb = Sandbox::new();
    sb.write_settings("{}");
    sb.tokred()
        .arg("configure")
        .assert()
        .failure()
        .stderr(predicate::str::contains("--yes"));
    assert_eq!(std::fs::read_to_string(sb.settings()).unwrap(), "{}");
}

#[test]
fn unknown_folder_without_a_terminal_suggests_the_flag() {
    let sb = Sandbox::new();
    let mut cmd = sb.tokred();
    cmd.env("CLAUDE_CONFIG_DIR", sb.root.path().join("missing"))
        .args(["configure", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--claude-dir"));
}

#[test]
fn explicit_invalid_claude_dir_is_an_error() {
    let sb = Sandbox::new();
    let empty = sb.root.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    sb.tokred()
        .args(["configure", "--yes", "--claude-dir"])
        .arg(&empty)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a Claude Code folder"));
}

#[test]
fn purge_refuses_folders_that_do_not_look_like_tokred_data() {
    let sb = Sandbox::new();
    sb.write_settings("{}");
    std::fs::create_dir_all(sb.data()).unwrap();
    std::fs::write(sb.data().join("precious.txt"), "keep me").unwrap();
    sb.tokred()
        .args(["unconfigure", "--purge"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("refusing to purge"));
    assert!(sb.data().join("precious.txt").exists());
}

/// Hooks must never fail or print anything in F0, whatever they receive.
#[test]
fn hooks_are_fail_open_stubs() {
    let sb = Sandbox::new();
    let stdin = r#"{"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Read"}"#;
    for event in [
        "route",
        "enforce",
        "guard-read",
        "guard-search",
        "session-check",
        "session-start",
    ] {
        sb.tokred()
            .args(["hook", event])
            .write_stdin(stdin)
            .assert()
            .success()
            .stdout("");
    }
    // Garbage stdin, unknown event, and no event at all still exit 0 and stay silent.
    sb.tokred()
        .args(["hook", "guard-read"])
        .write_stdin("\u{0}not json")
        .assert()
        .success()
        .stdout("");
    sb.tokred()
        .args(["hook", "from-the-future"])
        .write_stdin(stdin)
        .assert()
        .success()
        .stdout("");
    sb.tokred().arg("hook").assert().success().stdout("");
}

#[test]
fn edited_agent_survives_reconfigure_and_doctor_reports_it() {
    let sb = Sandbox::new();
    sb.write_settings("{}");
    sb.tokred().args(["configure", "--yes"]).assert().success();
    let agent = sb.claude().join("agents/tokred-dev.md");
    std::fs::write(&agent, "my own version").unwrap();

    sb.tokred().args(["configure", "--yes"]).assert().success();
    assert_eq!(std::fs::read_to_string(&agent).unwrap(), "my own version");
    sb.tokred()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("edited by you"));
}
