//! `env.ANTHROPIC_BASE_URL` in settings.json, against real files in temporary directories.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use serde_json::{Value, json};
use tokred_install::StdFileSystem;
use tokred_install::base_url::{BASE_URL_KEY, read_base_url, write_base_url};
use tokred_install::error::{InstallError, MergeError};

const URL: &str = "http://127.0.0.1:1029";

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name);
    std::fs::read_to_string(&path).unwrap()
}

struct Dir {
    _tmp: tempfile::TempDir,
    claude: std::path::PathBuf,
}

impl Dir {
    /// A Claude Code folder (with a space in its name) holding `settings` when given.
    fn new(settings: Option<&str>) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join("my claude");
        if let Some(text) = settings {
            std::fs::create_dir_all(&claude).unwrap();
            std::fs::write(claude.join("settings.json"), text).unwrap();
        }
        Self { _tmp: tmp, claude }
    }

    fn text(&self) -> String {
        std::fs::read_to_string(self.claude.join("settings.json")).unwrap()
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.text()).unwrap()
    }

    fn write(&self, desired: Option<&str>) -> Result<bool, InstallError> {
        write_base_url(&StdFileSystem, &self.claude, desired)
    }

    fn read(&self) -> Option<String> {
        read_base_url(&StdFileSystem, &self.claude).unwrap()
    }
}

#[test]
fn a_missing_file_is_created_with_only_the_url_and_removed_again() {
    let dir = Dir::new(None);
    assert_eq!(dir.read(), None);
    assert!(
        !dir.write(None).unwrap(),
        "nothing to remove, nothing written"
    );
    assert!(!dir.claude.exists(), "removing never creates the folder");

    assert!(dir.write(Some(URL)).unwrap());
    assert_eq!(dir.json(), json!({"env": {BASE_URL_KEY: URL}}));
    assert_eq!(dir.read().as_deref(), Some(URL));
    assert!(!dir.write(Some(URL)).unwrap(), "idempotent");

    assert!(dir.write(None).unwrap());
    assert_eq!(dir.json(), json!({}));
}

#[test]
fn other_env_keys_and_settings_survive_a_set_and_a_remove() {
    let original = fixture("settings-env-other-keys.json");
    let dir = Dir::new(Some(&original));
    assert!(dir.write(Some(URL)).unwrap());
    let with_url = dir.json();
    assert_eq!(with_url["env"]["FOO"], "1");
    assert_eq!(with_url["env"]["BAR"], "2");
    assert_eq!(with_url["env"][BASE_URL_KEY], URL);
    assert_eq!(with_url["permissions"]["allow"][0], "Bash(ls)");

    assert!(dir.write(None).unwrap());
    assert_eq!(dir.text(), original, "back to the very same text");
}

#[test]
fn a_file_that_already_says_so_is_not_rewritten() {
    // Compact formatting: a rewrite would pretty-print it.
    let ours = fixture("settings-base-url-ours.json");
    let dir = Dir::new(Some(&ours));
    assert!(!dir.write(Some(URL)).unwrap());
    assert_eq!(dir.text(), ours);

    let plain = "{ \"model\":\"opus\" }";
    let dir = Dir::new(Some(plain));
    assert!(!dir.write(None).unwrap());
    assert_eq!(dir.text(), plain);
}

#[test]
fn another_port_replaces_the_value_in_place() {
    let dir = Dir::new(Some(&fixture("settings-base-url-ours.json")));
    assert!(dir.write(Some("http://127.0.0.1:2000")).unwrap());
    assert_eq!(dir.read().as_deref(), Some("http://127.0.0.1:2000"));
    assert_eq!(dir.json()["model"], "opus");
}

#[test]
fn a_foreign_value_can_be_read_and_the_other_keys_stay_when_it_is_removed() {
    let dir = Dir::new(Some(&fixture("settings-base-url-foreign.json")));
    assert_eq!(dir.read().as_deref(), Some("https://gateway.example.com"));
    assert!(dir.write(None).unwrap());
    assert_eq!(dir.json(), json!({"model": "opus", "env": {"FOO": "1"}}));
}

#[test]
fn files_that_cannot_be_merged_are_never_written() {
    let invalid = fixture("settings-invalid.json");
    let dir = Dir::new(Some(&invalid));
    assert!(matches!(
        dir.write(Some(URL)),
        Err(InstallError::InvalidJson { .. })
    ));
    assert!(read_base_url(&StdFileSystem, &dir.claude).is_err());
    assert_eq!(dir.text(), invalid);

    for (text, expected) in [
        ("[1]", MergeError::RootNotObject),
        ("{\"env\": 3}", MergeError::EnvNotObject),
    ] {
        let dir = Dir::new(Some(text));
        let err = dir.write(Some(URL)).unwrap_err();
        assert!(
            matches!(&err, InstallError::Merge { source, .. } if *source == expected),
            "{err}"
        );
        assert_eq!(dir.text(), text);
    }
}
