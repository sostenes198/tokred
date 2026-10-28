//! Finding a stable path for the tokred binary.

use std::path::Path;

/// The path to write in hook commands.
#[derive(Debug, Clone)]
pub struct BinaryPath {
    /// The path, as a string (quoted by the caller).
    pub path: String,
    /// `true` when it is the path found on `PATH` (survives package-manager upgrades);
    /// `false` when it is the current executable's own path.
    pub stable: bool,
}

/// Locates `tokred` on `PATH` and checks it is the running executable. Package managers install a
/// shim or symlink (`/opt/homebrew/bin/tokred`) that survives upgrades, unlike the versioned
/// target (`.../Cellar/tokred/0.3.2/bin/tokred`) that `current_exe()` returns after resolution.
pub fn find() -> anyhow::Result<BinaryPath> {
    let current = std::env::current_exe()?;
    let current_canonical = current.canonicalize().unwrap_or_else(|_| current.clone());
    if let Ok(candidates) = which::which_all("tokred") {
        for candidate in candidates {
            let same = candidate
                .canonicalize()
                .is_ok_and(|canonical| canonical == current_canonical);
            if same {
                return Ok(BinaryPath {
                    path: display(&candidate),
                    stable: true,
                });
            }
        }
    }
    Ok(BinaryPath {
        path: display(&current),
        stable: false,
    })
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Whether an `rtk` executable is on `PATH`.
pub fn rtk_on_path() -> bool {
    which::which("rtk").is_ok()
}
