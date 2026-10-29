//! How tokred was installed, judged from where its own executable lives.
//!
//! The install script puts the binary in one folder, `$TOKRED_HOME/bin` (`~/.tokred/bin` by
//! default, `%LOCALAPPDATA%\tokred\bin` on Windows). A binary there updates itself; a binary
//! anywhere else (a build from source, a copy) is told to reinstall with the script.

use std::path::{Path, PathBuf};

/// Where the running binary came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    /// The install script: the binary sits in the script's folder and updates itself.
    Script,
    /// Anything else (built from source, copied by hand): reinstall with the script.
    Other,
}

/// The folder the install script puts the binary in.
///
/// `tokred_home` is `$TOKRED_HOME`, `home` the user's home folder and `local_app_data`
/// `%LOCALAPPDATA%`; empty values count as unset. Returns `None` when nothing is known.
#[must_use]
pub fn install_bin_dir(
    tokred_home: Option<PathBuf>,
    home: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    windows: bool,
) -> Option<PathBuf> {
    let not_empty = |path: PathBuf| (!path.as_os_str().is_empty()).then_some(path);
    let root = tokred_home.and_then(not_empty).or_else(|| {
        if windows {
            local_app_data
                .and_then(not_empty)
                .map(|dir| dir.join("tokred"))
        } else {
            home.and_then(not_empty).map(|dir| dir.join(".tokred"))
        }
    });
    root.map(|root| root.join("bin"))
}

/// Detects the install method: [`InstallMethod::Script`] when the executable's folder is the
/// install script's folder (`install_bin`). Both paths are resolved first, so a symlinked home
/// (macOS maps `/var` to `/private/var`) does not hide the match.
#[must_use]
pub fn detect(exe: &Path, install_bin: Option<&Path>) -> InstallMethod {
    let resolve = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let in_install_folder = install_bin
        .zip(exe.parent())
        .is_some_and(|(bin, parent)| resolve(bin) == resolve(parent));
    if in_install_folder {
        InstallMethod::Script
    } else {
        InstallMethod::Other
    }
}

/// What to tell the user to do about a new version.
#[must_use]
pub const fn upgrade_hint(method: InstallMethod) -> &'static str {
    match method {
        InstallMethod::Script => "run: tokred update",
        InstallMethod::Other => "reinstall with the install script (see the README)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::unnecessary_wraps)] // reads better than `Some(PathBuf::from(..))` x10
    fn some(path: &str) -> Option<PathBuf> {
        Some(PathBuf::from(path))
    }

    #[test]
    fn the_install_folder_follows_tokred_home_then_the_platform() {
        assert_eq!(
            install_bin_dir(some("/opt/t"), some("/home/u"), None, false),
            some("/opt/t/bin")
        );
        assert_eq!(
            install_bin_dir(None, some("/home/u"), some("/ignored"), false),
            some("/home/u/.tokred/bin")
        );
        assert_eq!(
            install_bin_dir(
                Some(PathBuf::new()),
                some("/home/u"),
                some("/ignored"),
                false
            ),
            some("/home/u/.tokred/bin"),
            "an empty TOKRED_HOME counts as unset"
        );
        assert_eq!(
            install_bin_dir(None, some("/ignored"), some("/local"), true),
            some("/local/tokred/bin")
        );
        assert_eq!(install_bin_dir(None, None, None, false), None);
        assert_eq!(
            install_bin_dir(None, Some(PathBuf::new()), None, false),
            None
        );
        assert_eq!(install_bin_dir(None, some("/h"), None, true), None);
    }

    #[test]
    fn a_binary_in_the_install_folder_is_a_script_install() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("tokred");
        std::fs::write(&exe, "x").unwrap();
        assert_eq!(detect(&exe, Some(&bin)), InstallMethod::Script);
        // The same folder spelled differently.
        let roundabout = dir.path().join("bin/../bin");
        assert_eq!(detect(&exe, Some(&roundabout)), InstallMethod::Script);
    }

    #[test]
    fn a_binary_anywhere_else_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let other = dir.path().join("target/debug");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let exe = other.join("tokred");
        assert_eq!(detect(&exe, Some(&bin)), InstallMethod::Other);
        assert_eq!(detect(&exe, None), InstallMethod::Other);
        assert_eq!(
            detect(Path::new("tokred"), Some(&bin)),
            InstallMethod::Other
        );
        // A folder that does not exist is compared as written.
        let ghost = Path::new("/no/such/place/bin");
        assert_eq!(
            detect(&ghost.join("tokred"), Some(ghost)),
            InstallMethod::Script
        );
        assert_eq!(
            detect(&ghost.join("tokred"), Some(&bin)),
            InstallMethod::Other
        );
    }

    #[test]
    fn the_hint_names_the_way_to_upgrade() {
        assert_eq!(upgrade_hint(InstallMethod::Script), "run: tokred update");
        assert!(upgrade_hint(InstallMethod::Other).contains("install script"));
    }
}
