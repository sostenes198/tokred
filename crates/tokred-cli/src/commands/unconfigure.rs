//! `tokred unconfigure`.

use std::path::PathBuf;

use anyhow::{Result, bail};
use tokred_core::{FileSystem, ManifestRepo};
use tokred_install::{StdFileSystem, unconfigure};

use super::{existing_store, select_claude_dir, show};
use crate::env::Env;

pub fn run(env: &Env, dir_flag: Option<PathBuf>, purge: bool) -> Result<()> {
    let fs = StdFileSystem;
    let claude_dir = select_claude_dir(&fs, env, dir_flag, false)?;
    let store = existing_store(env)?;
    let report = unconfigure(
        &fs,
        store.as_ref().map(|s| s as &dyn ManifestRepo),
        &claude_dir,
    )?;
    // Release the database file before a possible purge (required on Windows).
    drop(store);

    println!("tokred unconfigure ({})", show(&claude_dir));
    println!(
        "  hooks removed from settings.json: {}",
        report.hooks_removed
    );
    println!("  agents removed: {}", report.agents_removed.len());
    for kept in &report.agents_kept {
        println!(
            "  kept {} (edited by you, or its origin cannot be verified)",
            show(kept)
        );
    }
    if !report.conflict_files_removed.is_empty() {
        println!(
            "  .new files removed: {}",
            report.conflict_files_removed.len()
        );
    }

    if purge {
        let root = env.data.root();
        if !fs.exists(root) {
            println!(
                "  data folder {} does not exist; nothing to purge",
                show(root)
            );
        } else if !(fs.exists(&env.data.db_path()) || fs.exists(&env.data.config_path())) {
            bail!(
                "refusing to purge {}: it does not look like a tokred data folder",
                show(root)
            );
        } else {
            fs.remove_dir_all(root)?;
            println!("  data folder deleted: {}", show(root));
        }
    }
    println!(
        "\nThe tokred binary itself is removed with your package manager (brew, winget, scoop, cargo)."
    );
    Ok(())
}
