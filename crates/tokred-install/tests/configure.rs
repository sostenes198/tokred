//! Integration tests: configure/unconfigure against real files in temporary directories.
//! They never touch the real `~/.claude` or the real data folder.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use serde_json::Value;
use tokred_core::{Clock, FileSystem, ManifestRepo, Timestamp};
use tokred_install::agents::{TEMPLATES, conflict_path};
use tokred_install::settings::{remove_tokred, tokred_entries};
use tokred_install::{
    ConfigureInput, InstallError, StdFileSystem, apply_configure, doctor, plan_configure,
    unconfigure,
};
use tokred_store::Store;

const BINARY: &str = "/opt/bin/tokred";
const VERSION: &str = "0.1.0";

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp(1_700_000_000)
    }
}

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap()
}

struct Env {
    _tmp: tempfile::TempDir,
    claude: PathBuf,
    store: Store,
}

impl Env {
    fn new(settings: Option<&str>) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        // A directory with a space, like many Windows paths.
        let claude = tmp.path().join("my claude");
        std::fs::create_dir_all(claude.join("projects")).unwrap();
        if let Some(text) = settings {
            std::fs::write(claude.join("settings.json"), text).unwrap();
        }
        let store = Store::open(&tmp.path().join("tokred.db")).unwrap();
        Self {
            _tmp: tmp,
            claude,
            store,
        }
    }

    fn settings_path(&self) -> PathBuf {
        self.claude.join("settings.json")
    }

    fn input(&self) -> ConfigureInput<'_> {
        ConfigureInput {
            claude_dir: &self.claude,
            binary: BINARY,
            version: VERSION,
            force: false,
        }
    }

    fn configure(
        &self,
        input: &ConfigureInput<'_>,
    ) -> Result<tokred_install::ConfigureReport, InstallError> {
        let plan = plan_configure(&StdFileSystem, Some(&self.store), input)?;
        apply_configure(&StdFileSystem, &self.store, &FixedClock, &plan)
    }

    fn settings_json(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.settings_path()).unwrap()).unwrap()
    }
}

const VALID_FIXTURES: [&str; 5] = [
    "settings-empty.json",
    "settings-rtk.json",
    "settings-user-hooks.json",
    "settings-old-tokred.json",
    "settings-unknown-keys.json",
];

#[test]
fn configure_configure_unconfigure_returns_the_original_semantically() {
    for name in VALID_FIXTURES {
        let original_text = fixture(name);
        let original: Value = serde_json::from_str(&original_text).unwrap();
        let env = Env::new(Some(&original_text));

        env.configure(&env.input()).unwrap();
        let after_first = std::fs::read_to_string(env.settings_path()).unwrap();
        env.configure(&env.input()).unwrap();
        let after_second = std::fs::read_to_string(env.settings_path()).unwrap();
        assert_eq!(
            after_first, after_second,
            "{name}: configure is not idempotent (bytes)"
        );

        let entries = tokred_entries(&env.settings_json());
        assert_eq!(entries.len(), 6, "{name}");

        unconfigure(&StdFileSystem, Some(&env.store), &env.claude).unwrap();
        // Old tokred entries in the original are replaced, never restored; everything else
        // (including the pre-existing empty `Notification` event) must come back untouched.
        assert_eq!(env.settings_json(), remove_tokred(original), "{name}");
    }
}

#[test]
fn missing_settings_file_is_created_and_left_as_empty_object_after_unconfigure() {
    let env = Env::new(None);
    env.configure(&env.input()).unwrap();
    assert_eq!(tokred_entries(&env.settings_json()).len(), 6);
    assert!(
        !env.claude.join("settings.json.tokred.bak").exists(),
        "nothing to back up"
    );

    unconfigure(&StdFileSystem, Some(&env.store), &env.claude).unwrap();
    assert_eq!(env.settings_json(), serde_json::json!({}));
}

#[test]
fn invalid_json_aborts_without_writing_anything() {
    let broken = fixture("settings-invalid.json");
    let env = Env::new(Some(&broken));

    let err = env.configure(&env.input()).unwrap_err();
    assert!(matches!(err, InstallError::InvalidJson { .. }), "{err}");
    assert_eq!(
        std::fs::read_to_string(env.settings_path()).unwrap(),
        broken
    );
    assert!(!env.claude.join("settings.json.tokred.bak").exists());
    assert!(!env.claude.join("agents").exists());

    let err = unconfigure(&StdFileSystem, Some(&env.store), &env.claude).unwrap_err();
    assert!(matches!(err, InstallError::InvalidJson { .. }));
}

#[test]
fn backup_keeps_only_the_last_original() {
    let original = fixture("settings-rtk.json");
    let env = Env::new(Some(&original));
    env.configure(&env.input()).unwrap();
    let backup = env.claude.join("settings.json.tokred.bak");
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);

    // A second run with a different binary path changes settings again; the backup is replaced.
    let moved = ConfigureInput {
        binary: "/new/tokred",
        ..env.input()
    };
    env.configure(&moved).unwrap();
    assert_ne!(std::fs::read_to_string(&backup).unwrap(), original);
}

#[test]
fn rerun_with_nothing_to_do_does_not_rewrite_settings() {
    let env = Env::new(Some("{\n    \"model\":   \"opus\"\n}\n"));
    env.configure(&env.input()).unwrap();
    let report = env.configure(&env.input()).unwrap();
    assert!(!report.settings_written);
}

#[test]
fn agents_are_installed_with_manifest_and_marker() {
    let env = Env::new(Some("{}"));
    env.configure(&env.input()).unwrap();
    for template in TEMPLATES {
        let path = env.claude.join("agents").join(template.file_name);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("<!-- managed-by: tokred v0.1.0 -->"));
        let entry = env.store.get(&path).unwrap().unwrap();
        assert_eq!(entry.version, VERSION);
        assert_eq!(
            entry.hash,
            blake3::hash(text.as_bytes()).to_hex().to_string()
        );
    }
}

#[test]
fn files_without_the_prefix_are_never_touched() {
    let env = Env::new(Some("{}"));
    let agents = env.claude.join("agents");
    std::fs::create_dir_all(&agents).unwrap();
    let mine = agents.join("my-agent.md");
    std::fs::write(&mine, "mine").unwrap();
    env.configure(&env.input()).unwrap();
    unconfigure(&StdFileSystem, Some(&env.store), &env.claude).unwrap();
    assert_eq!(std::fs::read_to_string(&mine).unwrap(), "mine");
}

#[test]
fn edited_agent_is_kept_and_a_new_file_is_written_unless_forced() {
    let env = Env::new(Some("{}"));
    env.configure(&env.input()).unwrap();
    let path = env.claude.join("agents/tokred-dev.md");
    std::fs::write(&path, "my edits").unwrap();

    // A new version arrives.
    let upgraded = ConfigureInput {
        version: "0.2.0",
        ..env.input()
    };
    env.configure(&upgraded).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "my edits");
    let new_file = conflict_path(&path);
    assert!(
        std::fs::read_to_string(&new_file)
            .unwrap()
            .contains("tokred v0.2.0")
    );

    // Untouched agents were updated normally.
    let other = std::fs::read_to_string(env.claude.join("agents/tokred-explorer.md")).unwrap();
    assert!(other.contains("tokred v0.2.0"));

    // doctor reports the conflict.
    let report = doctor(
        &StdFileSystem,
        Some(&env.store),
        &env.claude,
        BINARY,
        "0.2.0",
    )
    .unwrap();
    let dev = report
        .agents
        .iter()
        .find(|a| a.file == "tokred-dev.md")
        .unwrap();
    assert_eq!(dev.state, tokred_install::doctor::AgentState::Modified);
    assert!(dev.pending_new);

    // --force overwrites and cleans up the .new file.
    let forced = ConfigureInput {
        version: "0.2.0",
        force: true,
        ..env.input()
    };
    env.configure(&forced).unwrap();
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("tokred v0.2.0")
    );
    assert!(!new_file.exists());
}

#[test]
fn unconfigure_keeps_edited_agents_and_removes_the_rest() {
    let env = Env::new(Some("{}"));
    env.configure(&env.input()).unwrap();
    let edited = env.claude.join("agents/tokred-dev.md");
    std::fs::write(&edited, "my edits").unwrap();

    let report = unconfigure(&StdFileSystem, Some(&env.store), &env.claude).unwrap();
    assert_eq!(report.agents_kept, vec![edited.clone()]);
    assert_eq!(report.agents_removed.len(), 5);
    assert!(edited.exists());
    assert!(!env.claude.join("agents/tokred-explorer.md").exists());
    assert_eq!(
        env.store.list().unwrap().len(),
        1,
        "edited file stays in the manifest"
    );
}

#[test]
fn doctor_flags_missing_binary_and_detects_rtk() {
    let env = Env::new(Some(&fixture("settings-rtk.json")));
    env.configure(&env.input()).unwrap();
    let report = doctor(
        &StdFileSystem,
        Some(&env.store),
        &env.claude,
        BINARY,
        VERSION,
    )
    .unwrap();
    assert!(report.rtk_hooks_detected);
    assert!(
        report
            .hooks
            .iter()
            .all(|h| h.state == tokred_install::doctor::HookState::Ok)
    );
    // /opt/bin/tokred does not exist on the test machine.
    assert!(report.hooks.iter().all(|h| h.binary_missing));
    assert!(report.issues.iter().any(|i| i.contains("no longer exists")));
    assert!(!report.issues.iter().any(|i| i.contains("matches Bash")));
}

#[test]
fn std_file_system_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a b.txt");
    assert_eq!(StdFileSystem.read_to_string(&path).unwrap(), None);
    StdFileSystem.write_atomic(&path, b"one").unwrap();
    StdFileSystem.write_atomic(&path, b"two").unwrap();
    assert_eq!(
        StdFileSystem.read_to_string(&path).unwrap().as_deref(),
        Some("two")
    );
    assert!(StdFileSystem.remove_file(&path).unwrap());
    assert!(!StdFileSystem.remove_file(&path).unwrap());
    let leftovers: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
    assert!(leftovers.is_empty(), "temporary files must not leak");
}

#[test]
fn unmergeable_settings_are_reported_and_nothing_is_written() {
    let env = Env::new(Some(r#"{"hooks": 1}"#));
    let err = env.configure(&env.input()).unwrap_err();
    assert!(matches!(err, InstallError::Merge { .. }), "{err}");
    assert!(err.to_string().contains("nothing was changed"));
    assert!(!env.claude.join("agents").exists());

    // A malformed event that tokred does not write to does not stop it from removing its own
    // entries: unconfigure leaves what it does not understand alone.
    let mixed = r#"{"hooks": {"Stop": [{"hooks": [{"type": "command",
        "command": "\"/x/tokred\" hook session-check"}]}], "Bad": 1}}"#;
    let env = Env::new(Some(mixed));
    let report = unconfigure(&StdFileSystem, Some(&env.store), &env.claude).unwrap();
    assert_eq!(report.hooks_removed, 1);
    assert_eq!(
        env.settings_json(),
        serde_json::json!({"hooks": {"Bad": 1}})
    );
}

#[test]
fn identical_agents_with_a_lost_manifest_are_recorded_again() {
    let env = Env::new(Some("{}"));
    env.configure(&env.input()).unwrap();
    for entry in env.store.list().unwrap() {
        env.store.remove(&entry.path).unwrap();
    }
    assert!(env.store.list().unwrap().is_empty());

    let report = env.configure(&env.input()).unwrap();
    assert!(
        report
            .agents
            .iter()
            .all(|(_, action)| *action == tokred_install::agents::AgentAction::Unchanged)
    );
    assert_eq!(env.store.list().unwrap().len(), TEMPLATES.len());
}

#[test]
fn unconfigure_removes_retired_agents_listed_in_the_manifest_and_stale_new_files() {
    let env = Env::new(Some("{}"));
    env.configure(&env.input()).unwrap();

    // An agent that an older tokred installed and this one no longer ships.
    let retired = env.claude.join("agents/tokred-retired.md");
    std::fs::write(&retired, "old agent").unwrap();
    env.store
        .upsert(&tokred_core::ManifestEntry {
            path: retired.clone(),
            hash: blake3::hash(b"old agent").to_hex().to_string(),
            version: "0.0.1".into(),
            installed_at: Timestamp(1),
        })
        .unwrap();
    // A manifest row for something outside agents/ must be ignored.
    env.store
        .upsert(&tokred_core::ManifestEntry {
            path: env.claude.join("elsewhere/tokred-x.md"),
            hash: "h".into(),
            version: "0.0.1".into(),
            installed_at: Timestamp(1),
        })
        .unwrap();
    // A conflict file left by an earlier configure.
    let edited = env.claude.join("agents/tokred-dev.md");
    std::fs::write(&edited, "mine").unwrap();
    env.configure(&ConfigureInput {
        version: "0.2.0",
        ..env.input()
    })
    .unwrap();
    assert!(conflict_path(&edited).exists());

    let report = unconfigure(&StdFileSystem, Some(&env.store), &env.claude).unwrap();
    assert!(report.agents_removed.contains(&retired));
    assert!(!retired.exists());
    assert_eq!(report.conflict_files_removed, vec![conflict_path(&edited)]);
    assert_eq!(report.agents_kept, vec![edited]);
}

#[test]
fn unconfigure_without_a_database_keeps_every_agent() {
    let env = Env::new(Some("{}"));
    env.configure(&env.input()).unwrap();
    let report = unconfigure(&StdFileSystem, None, &env.claude).unwrap();
    assert!(report.agents_removed.is_empty());
    assert_eq!(report.agents_kept.len(), TEMPLATES.len());
}

#[test]
fn unconfigure_on_a_missing_settings_file_does_nothing() {
    let env = Env::new(None);
    let report = unconfigure(&StdFileSystem, Some(&env.store), &env.claude).unwrap();
    assert_eq!(report.hooks_removed, 0);
    assert!(!env.settings_path().exists());
}

// ---- doctor ----

use tokred_install::doctor::{AgentState, HookState};

#[test]
fn doctor_on_a_fresh_folder_reports_everything_missing() {
    let env = Env::new(None);
    let report = doctor(&StdFileSystem, None, &env.claude, BINARY, VERSION).unwrap();
    assert!(!report.settings_exists && report.settings_valid);
    assert!(
        report
            .hooks
            .iter()
            .all(|h| h.state == HookState::Missing && !h.binary_missing)
    );
    assert!(
        report
            .agents
            .iter()
            .all(|a| a.state == AgentState::Missing && !a.pending_new)
    );
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.contains("hook `guard-read` is not installed"))
    );
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.contains("agent `tokred-dev.md` is not installed"))
    );
    assert!(report.notes.iter().any(|n| n.contains("tokred configure")));
    assert!(!report.rtk_hooks_detected);
}

#[test]
fn doctor_survives_invalid_json_but_not_unreadable_settings() {
    let env = Env::new(Some("{ nope"));
    let report = doctor(
        &StdFileSystem,
        Some(&env.store),
        &env.claude,
        BINARY,
        VERSION,
    )
    .unwrap();
    assert!(report.settings_exists && !report.settings_valid);
    assert!(report.issues.iter().any(|i| i.contains("not valid JSON")));

    // settings.json is a directory: a real I/O problem, reported as an error.
    let env = Env::new(None);
    std::fs::create_dir_all(env.settings_path()).unwrap();
    let err = doctor(&StdFileSystem, None, &env.claude, BINARY, VERSION).unwrap_err();
    assert!(matches!(err, InstallError::Fs(_)), "{err}");
}

#[test]
fn doctor_detects_outdated_hooks_and_agents() {
    let env = Env::new(Some("{}"));
    env.configure(&ConfigureInput {
        binary: "/old/tokred",
        ..env.input()
    })
    .unwrap();
    let report = doctor(
        &StdFileSystem,
        Some(&env.store),
        &env.claude,
        BINARY,
        "0.2.0",
    )
    .unwrap();
    assert!(report.hooks.iter().all(|h| h.state == HookState::Outdated));
    assert!(
        report
            .agents
            .iter()
            .all(|a| a.state == AgentState::Outdated)
    );
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.contains("hook `route` is outdated"))
    );
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.contains("agent `tokred-planner.md` is outdated"))
    );
}

#[test]
fn doctor_handles_unquoted_commands_and_overlaps() {
    let settings = r#"{"hooks": {"PreToolUse": [
        {"matcher": "Read", "hooks": [{"type": "command", "command": "python3 audit.py"}]},
        {"matcher": "Read", "hooks": [{"type": "command", "command": "tokred hook guard-read"}]},
        {"matcher": "Bash|Read", "hooks": [{"type": "command", "command": "\"/x/tokred\" hook enforce"}]}
    ]}}"#;
    let env = Env::new(Some(settings));
    let report = doctor(
        &StdFileSystem,
        Some(&env.store),
        &env.claude,
        BINARY,
        VERSION,
    )
    .unwrap();

    let read = report
        .hooks
        .iter()
        .find(|h| h.hook == "guard-read")
        .unwrap();
    assert!(
        read.binary_missing,
        "a bare `tokred` is not an existing file path"
    );
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.contains("`enforce` matches Bash"))
    );
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.contains("shares the `Read` matcher of `guard-read`"))
    );
}

// ---- file system adapter ----

#[test]
fn std_file_system_reports_real_io_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("a dir");
    std::fs::create_dir(&dir).unwrap();
    let file = tmp.path().join("a file");
    std::fs::write(&file, "x").unwrap();

    // Reading, deleting or writing through the wrong kind of path fails with the path in the error.
    let err = StdFileSystem.read_to_string(&dir).unwrap_err();
    assert!(err.to_string().contains("a dir"));
    assert!(StdFileSystem.remove_file(&dir).is_err());
    assert!(StdFileSystem.remove_dir_all(&file).is_err());
    assert!(StdFileSystem.create_dir_all(&file.join("sub")).is_err());
    let missing_parent = tmp.path().join("nope").join("x.txt");
    assert!(StdFileSystem.write_atomic(&missing_parent, b"x").is_err());

    // Missing things are not errors for removal.
    assert!(
        !StdFileSystem
            .remove_dir_all(&tmp.path().join("ghost"))
            .unwrap()
    );
    assert!(StdFileSystem.remove_dir_all(&dir).unwrap());
    assert!(!StdFileSystem.exists(&dir));
    assert!(StdFileSystem.is_dir(tmp.path()));
}
