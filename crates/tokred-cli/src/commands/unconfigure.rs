//! `tokred unconfigure`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use tokred_core::{FileSystem, ManifestRepo};
use tokred_install::{StdFileSystem, UnconfigureReport, unconfigure};

use super::{existing_store, select_claude_dir, show};
use crate::env::Env;
use crate::io::Io;

const FOOTER: &str = "The tokred binary and its shell setup stay: `tokred uninstall` removes the proxy and its service as well, and the uninstall script removes the binary (see docs/installation).";

pub fn run(env: &Env, io: &mut Io, dir_flag: Option<PathBuf>, purge: bool) -> Result<()> {
    let claude_dir = select_claude_dir(&StdFileSystem, env, io, dir_flag, false)?;
    apply(env, io, &claude_dir, purge)?;
    writeln!(io.out(), "\n{FOOTER}")?;
    Ok(())
}

/// Takes tokred out of `claude_dir` and, with `purge`, deletes the data folder.
pub fn apply(env: &Env, io: &mut Io, claude_dir: &Path, purge: bool) -> Result<()> {
    let fs = StdFileSystem;
    let store = existing_store(env)?;
    let report = unconfigure(
        &fs,
        store.as_ref().map(|s| s as &dyn ManifestRepo),
        claude_dir,
    )?;
    // Release the database file before a possible purge (required on Windows).
    drop(store);

    let shown = render_report(claude_dir, &report);
    write!(io.out(), "{shown}")?;
    if purge {
        purge_data(env, io)?;
    }
    Ok(())
}

/// Deletes the data folder (when it is tokred's) and says so.
pub fn purge_data(env: &Env, io: &mut Io) -> Result<()> {
    let outcome = purge_data_folder(&StdFileSystem, env)?;
    writeln!(io.out(), "  {outcome}")?;
    Ok(())
}

fn render_report(claude_dir: &Path, report: &UnconfigureReport) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "tokred unconfigure ({})", show(claude_dir));
    let _ = writeln!(
        out,
        "  hooks removed from settings.json: {}",
        report.hooks_removed
    );
    let _ = writeln!(out, "  agents removed: {}", report.agents_removed.len());
    for kept in &report.agents_kept {
        let _ = writeln!(
            out,
            "  kept {} (edited by you, or its origin cannot be verified)",
            show(kept)
        );
    }
    if !report.conflict_files_removed.is_empty() {
        let _ = writeln!(
            out,
            "  .new files removed: {}",
            report.conflict_files_removed.len()
        );
    }
    out
}

/// Deletes the data folder, but only if it looks like tokred's.
fn purge_data_folder(fs: &dyn FileSystem, env: &Env) -> Result<String> {
    let root = env.data.root();
    if !fs.exists(root) {
        return Ok(format!(
            "data folder {} does not exist; nothing to purge",
            show(root)
        ));
    }
    if !(fs.exists(&env.data.db_path()) || fs.exists(&env.data.config_path())) {
        bail!(
            "refusing to purge {}: it does not look like a tokred data folder",
            show(root)
        );
    }
    fs.remove_dir_all(root)?;
    Ok(format!("data folder deleted: {}", show(root)))
}

#[cfg(test)]
mod tests {
    use super::super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted;

    fn configured() -> Sandbox {
        let sb = Sandbox::new();
        let (mut io, _) = scripted(false, "");
        super::super::configure::run(&sb.env, &mut io, Some(sb.claude.clone()), true, false)
            .unwrap();
        sb
    }

    fn unconfigure_cmd(sb: &Sandbox, purge: bool) -> (Result<()>, String) {
        let (mut io, out) = scripted(false, "");
        let result = run(&sb.env, &mut io, Some(sb.claude.clone()), purge);
        (result, out.text())
    }

    #[test]
    fn removes_hooks_and_agents() {
        let sb = configured();
        let (result, out) = unconfigure_cmd(&sb, false);
        result.unwrap();
        assert!(out.contains("hooks removed from settings.json: 6"));
        assert!(out.contains("agents removed: 6"));
        assert!(!out.contains("kept"));
        assert!(!sb.agent("tokred-dev.md").exists());
        assert!(sb.env.data.root().exists(), "no --purge, data stays");
    }

    #[test]
    fn reports_kept_agents_and_removed_conflict_files() {
        let sb = configured();
        std::fs::write(sb.agent("tokred-dev.md"), "my edits").unwrap();
        // Leave a stale .new file next to it, as an update would.
        std::fs::write(sb.agent("tokred-dev.md.new"), "new").unwrap();

        let (result, out) = unconfigure_cmd(&sb, false);
        result.unwrap();
        assert!(out.contains("kept "), "{out}");
        assert!(out.contains(".new files removed: 1"), "{out}");
        assert!(sb.agent("tokred-dev.md").exists());
    }

    #[test]
    fn purge_deletes_a_tokred_data_folder() {
        let sb = configured();
        let (result, out) = unconfigure_cmd(&sb, true);
        result.unwrap();
        assert!(out.contains("data folder deleted"));
        assert!(!sb.env.data.root().exists());
    }

    #[test]
    fn purge_with_no_data_folder_has_nothing_to_do() {
        let sb = Sandbox::new();
        sb.write_settings("{}");
        let (result, out) = unconfigure_cmd(&sb, true);
        result.unwrap();
        assert!(out.contains("nothing to purge"));
    }

    #[test]
    fn purge_refuses_a_folder_that_is_not_tokreds() {
        let sb = Sandbox::new();
        sb.write_settings("{}");
        std::fs::create_dir_all(sb.env.data.root()).unwrap();
        std::fs::write(sb.env.data.root().join("precious.txt"), "keep").unwrap();
        let (result, _) = unconfigure_cmd(&sb, true);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("refusing to purge")
        );
        assert!(sb.env.data.root().join("precious.txt").exists());
    }

    #[test]
    fn invalid_settings_json_is_an_error_and_changes_nothing() {
        let sb = Sandbox::new();
        sb.write_settings("{ nope");
        let (result, out) = unconfigure_cmd(&sb, false);
        assert!(result.unwrap_err().to_string().contains("not valid JSON"));
        assert_eq!(out, "");
        assert_eq!(std::fs::read_to_string(sb.settings()).unwrap(), "{ nope");
    }

    #[test]
    fn a_corrupt_database_is_an_error() {
        let sb = Sandbox::new();
        sb.write_settings("{}");
        std::fs::create_dir_all(sb.env.data.root()).unwrap();
        std::fs::write(sb.env.data.db_path(), "garbage").unwrap();
        let (result, _) = unconfigure_cmd(&sb, false);
        assert!(result.is_err());
    }
}
