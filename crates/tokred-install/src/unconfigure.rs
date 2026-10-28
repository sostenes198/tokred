//! `unconfigure`: removes only the parts tokred added to the Claude Code folder.

use std::path::{Path, PathBuf};

use tokred_core::{FileSystem, ManifestRepo};

use crate::agents::{FILE_PREFIX, TEMPLATES, agents_dir, conflict_path, content_hash};
use crate::configure::{backup_path, read_settings, serialize_settings, settings_path};
use crate::error::InstallError;
use crate::settings::{remove_tokred, tokred_entries};

/// Outcome of [`unconfigure`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnconfigureReport {
    /// Number of hook handlers removed from `settings.json`.
    pub hooks_removed: usize,
    /// Agent files deleted.
    pub agents_removed: Vec<PathBuf>,
    /// Agent files left in place because the user edited them (or tokred cannot prove it wrote
    /// them as they are).
    pub agents_kept: Vec<PathBuf>,
    /// `.new` conflict files deleted.
    pub conflict_files_removed: Vec<PathBuf>,
}

/// Removes tokred's hooks from `settings.json` and its unmodified agent files.
///
/// `manifest` is `None` when the database no longer exists; agent files are then never deleted
/// because their origin cannot be verified.
pub fn unconfigure(
    fs: &dyn FileSystem,
    manifest: Option<&dyn ManifestRepo>,
    claude_dir: &Path,
) -> Result<UnconfigureReport, InstallError> {
    let mut report = UnconfigureReport::default();

    let settings = settings_path(claude_dir);
    let (current, original) = read_settings(fs, &settings)?;
    let removed = tokred_entries(&current).len();
    if removed > 0 {
        let stripped = remove_tokred(current).map_err(|source| InstallError::Merge {
            path: settings.clone(),
            source,
        })?;
        if let Some(original) = original {
            fs.write_atomic(&backup_path(claude_dir), original.as_bytes())?;
        }
        fs.write_atomic(&settings, serialize_settings(&stripped)?.as_bytes())?;
        report.hooks_removed = removed;
    }

    let dir = agents_dir(claude_dir);
    let mut candidates: Vec<PathBuf> = TEMPLATES.iter().map(|t| dir.join(t.file_name)).collect();
    if let Some(repo) = manifest {
        for entry in repo.list()? {
            let in_agents = entry.path.parent() == Some(dir.as_path());
            let owned_name = entry
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(FILE_PREFIX));
            if in_agents && owned_name && !candidates.contains(&entry.path) {
                candidates.push(entry.path);
            }
        }
    }

    for path in candidates {
        let recorded = match manifest {
            Some(repo) => repo.get(&path)?,
            None => None,
        };
        match fs.read_to_string(&path)? {
            None => {}
            Some(text) => {
                let untouched = recorded
                    .as_ref()
                    .is_some_and(|e| e.hash == content_hash(text.as_bytes()));
                if untouched {
                    fs.remove_file(&path)?;
                    report.agents_removed.push(path.clone());
                } else {
                    report.agents_kept.push(path.clone());
                }
            }
        }
        if let (Some(repo), true) = (
            manifest,
            recorded.is_some() && !report.agents_kept.contains(&path),
        ) {
            repo.remove(&path)?;
        }
        let stale_new = conflict_path(&path);
        if fs.remove_file(&stale_new)? {
            report.conflict_files_removed.push(stale_new);
        }
    }
    Ok(report)
}
