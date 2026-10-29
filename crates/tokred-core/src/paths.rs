//! Pure path resolution rules (no environment or disk access).

use std::path::{Path, PathBuf};

use crate::ports::FileSystem;

/// Candidate Claude Code folders, in priority order:
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

/// Home folder used for the default Claude Code folder (`<home>/.claude`).
///
/// On Windows Claude Code uses `%USERPROFILE%\.claude`, so that variable wins over the OS lookup.
/// This also lets tests redirect the folder: the OS lookup ignores environment variables on
/// Windows, which would otherwise point a sandboxed run at the developer's real profile.
#[must_use]
pub fn home_dir(
    is_windows: bool,
    userprofile: Option<PathBuf>,
    os_home: Option<PathBuf>,
) -> Option<PathBuf> {
    if is_windows {
        userprofile.or(os_home)
    } else {
        os_home
    }
}

/// Platform default of the tokred data folder: `<base>/tokred`, where `<base>` is
/// `%APPDATA%` on Windows (`%APPDATA%\tokred`) and the OS data folder elsewhere.
#[must_use]
pub fn platform_data_dir(
    is_windows: bool,
    appdata: Option<PathBuf>,
    os_data_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    let base = if is_windows {
        appdata.or(os_data_dir)
    } else {
        os_data_dir
    };
    base.map(|dir| dir.join("tokred"))
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
    fn windows_prefers_userprofile_over_the_os_lookup() {
        let userprofile = Some(PathBuf::from("/sandbox"));
        let os_home = Some(PathBuf::from("/real/home"));
        assert_eq!(
            home_dir(true, userprofile.clone(), os_home.clone()),
            userprofile
        );
        assert_eq!(home_dir(true, None, os_home.clone()), os_home);
        // Elsewhere the OS lookup (which honours $HOME) is used and USERPROFILE is ignored.
        assert_eq!(home_dir(false, userprofile, os_home.clone()), os_home);
    }

    #[test]
    fn data_dir_is_appdata_on_windows_and_the_os_folder_elsewhere() {
        let appdata = Some(PathBuf::from("/appdata"));
        let os = Some(PathBuf::from("/os-data"));
        assert_eq!(
            platform_data_dir(true, appdata.clone(), os.clone()),
            Some("/appdata/tokred".into())
        );
        assert_eq!(
            platform_data_dir(true, None, os.clone()),
            Some("/os-data/tokred".into())
        );
        assert_eq!(
            platform_data_dir(false, appdata, os),
            Some("/os-data/tokred".into())
        );
        assert_eq!(platform_data_dir(false, None, None), None);
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
