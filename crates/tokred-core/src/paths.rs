//! Pure path resolution rules (no environment or disk access).

use std::path::{Path, PathBuf};

use crate::ports::FileSystem;

/// Candidate Claude Code folders, in priority order (spec section 5.0):
/// the `--claude-dir` flag, the saved config value, `CLAUDE_CONFIG_DIR`, then the default.
#[must_use]
pub fn claude_dir_candidates(
    flag: Option<PathBuf>,
    saved: Option<PathBuf>,
    env: Option<PathBuf>,
    default: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for candidate in [flag, saved, env, default].into_iter().flatten() {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

/// A Claude Code folder is valid when it exists and has `settings.json` or `projects/`.
#[must_use]
pub fn is_valid_claude_dir(fs: &dyn FileSystem, dir: &Path) -> bool {
    fs.is_dir(dir) && (fs.exists(&dir.join("settings.json")) || fs.is_dir(&dir.join("projects")))
}

/// Picks the tokred data folder: `TOKRED_DATA_DIR` when set, otherwise the platform default.
#[must_use]
pub fn resolve_data_dir(env: Option<PathBuf>, platform_default: PathBuf) -> PathBuf {
    env.unwrap_or(platform_default)
}

/// Locations inside the tokred data folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataLayout {
    root: PathBuf,
}

impl DataLayout {
    /// Creates the layout rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The data folder itself.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// SQLite database file.
    #[must_use]
    pub fn db_path(&self) -> PathBuf {
        self.root.join("tokred.db")
    }

    /// Configuration file.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.root.join("tokred.toml")
    }

    /// Log folder (daily rotation, 7 days kept).
    #[must_use]
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Folder of generated reports.
    #[must_use]
    pub fn reports_dir(&self) -> PathBuf {
        self.root.join("reports")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_keep_priority_and_drop_duplicates() {
        let got = claude_dir_candidates(
            Some("/a".into()),
            None,
            Some("/b".into()),
            Some("/a".into()),
        );
        assert_eq!(got, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    }

    #[test]
    fn env_overrides_platform_default() {
        assert_eq!(
            resolve_data_dir(Some("/x".into()), "/d".into()),
            PathBuf::from("/x")
        );
        assert_eq!(resolve_data_dir(None, "/d".into()), PathBuf::from("/d"));
    }

    #[test]
    fn layout_paths() {
        let layout = DataLayout::new("/data");
        assert_eq!(layout.db_path(), PathBuf::from("/data/tokred.db"));
        assert_eq!(layout.config_path(), PathBuf::from("/data/tokred.toml"));
        assert_eq!(layout.logs_dir(), PathBuf::from("/data/logs"));
        assert_eq!(layout.reports_dir(), PathBuf::from("/data/reports"));
    }
}
