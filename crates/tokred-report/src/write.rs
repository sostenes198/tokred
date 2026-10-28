//! Writing the report to disk and the error type of the crate.

use std::path::Path;

use tokred_core::{FileSystem, FsError};

/// Failure while writing or opening the report.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    /// The file could not be written.
    #[error(transparent)]
    Fs(#[from] FsError),
    /// The browser (or default application) could not be launched.
    #[error("could not open `{path}`: {message}")]
    Open {
        /// The file that was to be opened.
        path: String,
        /// What went wrong.
        message: String,
    },
}

/// Writes `html` to `path` atomically, creating the parent folder when needed.
///
/// The CLI always passes the same path (`<data>/reports/metrics.html`), so every run
/// overwrites the previous report.
pub fn write_report(fs: &dyn FileSystem, path: &Path, html: &str) -> Result<(), ReportError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs.create_dir_all(parent)?;
    }
    fs.write_atomic(path, html.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io;
    use std::path::PathBuf;

    use super::*;

    /// Records the calls and can fail on demand.
    #[derive(Default)]
    struct FakeFs {
        calls: RefCell<Vec<String>>,
        fail_create: bool,
        fail_write: bool,
    }

    fn denied(path: &Path) -> FsError {
        FsError::new(path, io::Error::from(io::ErrorKind::PermissionDenied))
    }

    impl FileSystem for FakeFs {
        fn read_to_string(&self, _: &Path) -> Result<Option<String>, FsError> {
            Ok(None)
        }
        fn write_atomic(&self, path: &Path, contents: &[u8]) -> Result<(), FsError> {
            if self.fail_write {
                return Err(denied(path));
            }
            let text = String::from_utf8_lossy(contents);
            self.calls
                .borrow_mut()
                .push(format!("write {} {text}", path.display()));
            Ok(())
        }
        fn remove_file(&self, _: &Path) -> Result<bool, FsError> {
            Ok(false)
        }
        fn remove_dir_all(&self, _: &Path) -> Result<bool, FsError> {
            Ok(false)
        }
        fn create_dir_all(&self, path: &Path) -> Result<(), FsError> {
            if self.fail_create {
                return Err(denied(path));
            }
            self.calls
                .borrow_mut()
                .push(format!("mkdir {}", path.display()));
            Ok(())
        }
        fn exists(&self, _: &Path) -> bool {
            false
        }
        fn is_dir(&self, _: &Path) -> bool {
            false
        }
    }

    #[test]
    fn creates_the_parent_folder_then_writes_atomically() {
        let fs = FakeFs::default();
        let path = PathBuf::from("data").join("reports").join("metrics.html");
        write_report(&fs, &path, "<html></html>").unwrap();
        let calls = fs.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].starts_with("mkdir "));
        assert!(calls[1].ends_with("metrics.html <html></html>"));
    }

    #[test]
    fn a_bare_file_name_has_no_folder_to_create() {
        let fs = FakeFs::default();
        write_report(&fs, Path::new("metrics.html"), "x").unwrap();
        assert_eq!(fs.calls.borrow().len(), 1);
    }

    #[test]
    fn folder_failures_are_reported() {
        let fs = FakeFs {
            fail_create: true,
            ..FakeFs::default()
        };
        let err = write_report(&fs, Path::new("a/b.html"), "x").unwrap_err();
        assert!(matches!(err, ReportError::Fs(_)));
        assert!(err.to_string().contains("file system error"));
    }

    #[test]
    fn write_failures_are_reported() {
        let fs = FakeFs {
            fail_write: true,
            ..FakeFs::default()
        };
        let err = write_report(&fs, Path::new("b.html"), "x").unwrap_err();
        assert!(matches!(err, ReportError::Fs(_)));
        // The unused read/remove/exists/is_dir methods of the fake are still exercised.
        assert_eq!(fs.read_to_string(Path::new("x")).unwrap(), None);
        assert!(!fs.remove_file(Path::new("x")).unwrap());
        assert!(!fs.remove_dir_all(Path::new("x")).unwrap());
        assert!(!fs.exists(Path::new("x")));
        assert!(!fs.is_dir(Path::new("x")));
    }
}
