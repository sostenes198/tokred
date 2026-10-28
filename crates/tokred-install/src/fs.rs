//! `std::fs` adapter for the [`FileSystem`] port.

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use tokred_core::{FileSystem, FsError};

/// The real file system.
#[derive(Debug, Clone, Copy, Default)]
pub struct StdFileSystem;

fn tmp_sibling(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(format!(".tokred-tmp-{}", std::process::id()));
    path.with_file_name(name)
}

impl FileSystem for StdFileSystem {
    fn read_to_string(&self, path: &Path) -> Result<Option<String>, FsError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(FsError::new(path, err)),
        }
    }

    fn write_atomic(&self, path: &Path, contents: &[u8]) -> Result<(), FsError> {
        let tmp = tmp_sibling(path);
        let result = (|| {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(contents)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, path)
        })();
        if let Err(err) = result {
            let _ = std::fs::remove_file(&tmp);
            return Err(FsError::new(path, err));
        }
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> Result<bool, FsError> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
            Err(err) => Err(FsError::new(path, err)),
        }
    }

    fn remove_dir_all(&self, path: &Path) -> Result<bool, FsError> {
        match std::fs::remove_dir_all(path) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
            Err(err) => Err(FsError::new(path, err)),
        }
    }

    fn create_dir_all(&self, path: &Path) -> Result<(), FsError> {
        std::fs::create_dir_all(path).map_err(|err| FsError::new(path, err))
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }
}
