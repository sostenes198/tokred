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
        self.sandboxed(cargo_bin_cmd!("tokred"))
    }

    /// Applies the sandbox environment to a command.
    fn sandboxed(&self, mut cmd: Command) -> Command {
        let home = self.root.path().join("home");
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
    assert_eq!(json["schema_version"], 2);
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

/// A copy of the binary placed on `PATH` (plus a fake `rtk`) in a folder of its own.
fn install_on_path(sb: &Sandbox) -> (PathBuf, PathBuf) {
    let bin_dir = sb.root.path().join("bin dir");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let suffix = std::env::consts::EXE_SUFFIX;
    let tokred = bin_dir.join(format!("tokred{suffix}"));
    std::fs::copy(env!("CARGO_BIN_EXE_tokred"), &tokred).unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_tokred"),
        bin_dir.join(format!("rtk{suffix}")),
    )
    .unwrap();
    (tokred, bin_dir)
}

#[test]
fn a_tokred_found_on_path_is_the_one_written_into_the_hooks() {
    let sb = Sandbox::new();
    sb.write_settings("{}");
    let (tokred, bin_dir) = install_on_path(&sb);

    let mut configure = sb.sandboxed(Command::new(&tokred));
    configure
        .env("PATH", &bin_dir)
        .args(["configure", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Binary: {}",
            tokred.display()
        )))
        .stdout(predicate::str::contains("not found on PATH").not())
        .stdout(predicate::str::contains("rtk detected"));
    let settings = std::fs::read_to_string(sb.settings()).unwrap();
    assert!(
        settings.contains("tokred") && settings.contains("bin dir"),
        "{settings}"
    );

    let mut doctor = sb.sandboxed(Command::new(&tokred));
    doctor
        .env("PATH", &bin_dir)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("rtk:         detected"))
        .stdout(predicate::str::contains("No problems found."))
        .stdout(predicate::str::contains("not found on PATH").not());
}

#[test]
fn doctor_without_rtk_says_so() {
    let sb = Sandbox::new();
    sb.write_settings("{}");
    let empty_path = sb.root.path().join("empty path");
    std::fs::create_dir_all(&empty_path).unwrap();
    sb.tokred()
        .env("PATH", &empty_path)
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("rtk:         not detected"));
}

#[test]
fn configure_detects_rtk_hooks_in_settings_even_without_the_binary_on_path() {
    let sb = Sandbox::new();
    sb.write_settings(&fixture("settings-rtk.json"));
    let empty_path = sb.root.path().join("empty path");
    std::fs::create_dir_all(&empty_path).unwrap();
    sb.tokred()
        .env("PATH", &empty_path)
        .args(["configure", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("rtk detected"));
}

#[test]
fn hooks_log_only_when_the_data_folder_exists() {
    let sb = Sandbox::new();
    sb.write_settings("{}");
    let stdin = r#"{"session_id":"s1"}"#;

    // No data folder (e.g. after --purge): the hook must not recreate it.
    sb.tokred()
        .args(["hook", "guard-read"])
        .write_stdin(stdin)
        .assert()
        .success();
    assert!(!sb.data().exists());

    // With a data folder the hook logs into it.
    sb.tokred().args(["configure", "--yes"]).assert().success();
    std::fs::remove_dir_all(sb.data().join("logs")).unwrap();
    sb.tokred()
        .env("TOKRED_LOG", "debug")
        .args(["hook", "guard-read"])
        .write_stdin(stdin)
        .assert()
        .success()
        .stdout("");
    assert!(sb.data().join("logs").is_dir());
}

#[test]
fn a_different_tokred_on_path_is_not_mistaken_for_the_running_one() {
    let sb = Sandbox::new();
    sb.write_settings("{}");
    let (other_tokred, bin_dir) = install_on_path(&sb);

    // Run the original binary (not the copy on PATH): the copy is a different file.
    sb.tokred()
        .env("PATH", &bin_dir)
        .args(["configure", "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not found on PATH"))
        .stdout(predicate::str::contains(other_tokred.display().to_string()).not());
}
