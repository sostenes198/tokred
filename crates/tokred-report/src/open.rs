//! Opening the report in the default browser.

use std::path::{Path, PathBuf};

use crate::write::ReportError;

/// Something that can open a file for the user.
pub trait Opener {
    /// Opens `path` with the default application (the browser, for an HTML file).
    fn open(&self, path: &Path) -> Result<(), ReportError>;
}

/// The function that actually launches the default application.
pub type Launcher = fn(PathBuf) -> Result<(), opener::OpenError>;

/// [`Opener`] backed by the `opener` crate.
#[derive(Debug, Clone, Copy)]
pub struct SystemOpener {
    launch: Launcher,
}

impl SystemOpener {
    /// An opener that launches the real default browser.
    #[must_use]
    pub fn new() -> Self {
        Self::with_launcher(opener::open::<PathBuf>)
    }

    /// An opener that uses `launch` instead of the real launcher (used by tests).
    #[must_use]
    pub const fn with_launcher(launch: Launcher) -> Self {
        Self { launch }
    }
}

impl Default for SystemOpener {
    fn default() -> Self {
        Self::new()
    }
}

impl Opener for SystemOpener {
    fn open(&self, path: &Path) -> Result<(), ReportError> {
        (self.launch)(path.to_path_buf()).map_err(|err| ReportError::Open {
            path: path.display().to_string(),
            message: err.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    #[allow(clippy::unnecessary_wraps)] // must match `Launcher`
    fn succeed(_: PathBuf) -> Result<(), opener::OpenError> {
        Ok(())
    }

    fn fail(_: PathBuf) -> Result<(), opener::OpenError> {
        Err(opener::OpenError::Io(io::Error::other("no browser")))
    }

    #[test]
    fn open_succeeds_through_the_launcher() {
        let opener = SystemOpener::with_launcher(succeed);
        assert!(opener.open(Path::new("metrics.html")).is_ok());
    }

    #[test]
    fn open_reports_launcher_failures_with_the_path() {
        let opener = SystemOpener::with_launcher(fail);
        let err = opener.open(Path::new("metrics.html")).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("metrics.html"));
        assert!(text.contains("no browser"));
    }

    #[test]
    fn the_default_opener_is_the_real_one() {
        // Only constructed, never called: calling it would start a browser.
        let _ = SystemOpener::default();
        let _ = SystemOpener::new();
    }
}
