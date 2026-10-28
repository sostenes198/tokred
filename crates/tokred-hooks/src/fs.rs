//! Real adapters of the hook ports: the `std::fs` implementation of [`FileMeta`] and the
//! system clock.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokred_core::{Clock, Timestamp};
use walkdir::WalkDir;

use crate::ports::{FileMeta, FileStat};

/// Directories that are not counted as part of a repository's size.
const UNCOUNTED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "__pycache__",
    ".next",
];

/// The real file system.
#[derive(Debug, Clone, Copy)]
pub struct StdFileMeta {
    /// Longest a file count may take before it is abandoned (the hook then allows the call).
    pub count_deadline: Duration,
}

impl Default for StdFileMeta {
    fn default() -> Self {
        Self {
            count_deadline: Duration::from_millis(100),
        }
    }
}

fn unix_seconds(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl FileMeta for StdFileMeta {
    fn stat(&self, path: &Path) -> io::Result<Option<FileStat>> {
        match std::fs::metadata(path) {
            Ok(meta) => Ok(Some(FileStat {
                size: meta.len(),
                is_dir: meta.is_dir(),
                mtime: meta.modified().map_or(0, unix_seconds),
            })),
            // `NotADirectory` (a path below a file) is also "does not exist" for our purposes.
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(None)
            }
            Err(err) => Err(err),
        }
    }

    fn hash(&self, path: &Path) -> io::Result<String> {
        let mut file = std::fs::File::open(path)?;
        let mut hasher = blake3::Hasher::new();
        io::copy(&mut file, &mut hasher)?;
        Ok(hasher.finalize().to_hex().to_string())
    }

    fn count_files(&self, root: &Path, cap: u64) -> io::Result<u64> {
        let started = Instant::now();
        let walker = WalkDir::new(root).into_iter().filter_entry(|entry| {
            !(entry.file_type().is_dir()
                && entry.depth() > 0
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| UNCOUNTED_DIRS.contains(&name)))
        });
        let mut count = 0u64;
        for entry in walker.flatten() {
            if entry.file_type().is_file() {
                count += 1;
                if count >= cap {
                    return Ok(cap);
                }
            }
            if started.elapsed() > self.count_deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "counting files took too long",
                ));
            }
        }
        Ok(count)
    }

    fn read_tail(&self, path: &Path, max_bytes: u64) -> io::Result<Option<Vec<u8>>> {
        let opened = std::fs::File::open(path);
        if matches!(&opened, Err(err) if err.kind() == io::ErrorKind::NotFound) {
            return Ok(None);
        }
        let mut file = opened?;
        let len = file.metadata()?.len();
        file.seek(SeekFrom::Start(len.saturating_sub(max_bytes)))?;
        let mut bytes = Vec::new();
        file.take(max_bytes).read_to_end(&mut bytes)?;
        Ok(Some(bytes))
    }
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp(unix_seconds(SystemTime::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "x").unwrap();
    }

    #[test]
    fn stat_reports_files_directories_and_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "hello").unwrap();
        let fs = StdFileMeta::default();

        let stat = fs.stat(&file).unwrap().unwrap();
        assert_eq!((stat.size, stat.is_dir), (5, false));
        assert!(stat.mtime > 1_577_836_800);
        assert!(fs.stat(dir.path()).unwrap().unwrap().is_dir);
        assert_eq!(fs.stat(&dir.path().join("missing")).unwrap(), None);
        // A path below a regular file is "not found" too.
        assert_eq!(fs.stat(&file.join("below")).unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn stat_reports_errors_other_than_not_found() {
        // A name longer than the file system allows is an error, not "missing".
        let path = std::path::PathBuf::from("/tmp").join("a".repeat(300));
        assert!(StdFileMeta::default().stat(&path).is_err());
    }

    #[test]
    fn the_hash_changes_with_the_content() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "one").unwrap();
        let fs = StdFileMeta::default();
        let first = fs.hash(&file).unwrap();
        assert_eq!(first.len(), 64);
        assert_eq!(first, fs.hash(&file).unwrap());
        std::fs::write(&file, "two").unwrap();
        assert_ne!(first, fs.hash(&file).unwrap());
        assert!(fs.hash(&dir.path().join("missing")).is_err());
    }

    #[test]
    fn files_are_counted_skipping_build_and_vendor_directories() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "a.rs",
            "src/b.rs",
            "src/deep/c.rs",
            "node_modules/x/y.js",
            "target/z",
            ".git/HEAD",
        ] {
            touch(&dir.path().join(name));
        }
        let fs = StdFileMeta::default();
        assert_eq!(fs.count_files(dir.path(), 100).unwrap(), 3);
        // The count stops at the cap.
        assert_eq!(fs.count_files(dir.path(), 2).unwrap(), 2);
        // A missing root counts nothing.
        assert_eq!(fs.count_files(&dir.path().join("nope"), 10).unwrap(), 0);
    }

    #[test]
    fn a_root_that_is_itself_a_skipped_name_is_still_counted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("build");
        touch(&root.join("a"));
        assert_eq!(StdFileMeta::default().count_files(&root, 10).unwrap(), 1);
    }

    #[test]
    fn a_slow_count_is_abandoned() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("a"));
        touch(&dir.path().join("b"));
        let fs = StdFileMeta {
            count_deadline: Duration::ZERO,
        };
        let err = fs.count_files(dir.path(), 100).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn the_tail_is_the_last_bytes_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("t.jsonl");
        std::fs::write(&file, "0123456789").unwrap();
        let fs = StdFileMeta::default();
        assert_eq!(fs.read_tail(&file, 4).unwrap().unwrap(), b"6789");
        assert_eq!(fs.read_tail(&file, 100).unwrap().unwrap(), b"0123456789");
        assert_eq!(fs.read_tail(&dir.path().join("missing"), 4).unwrap(), None);
        // A directory cannot be read as a file.
        assert!(fs.read_tail(dir.path(), 4).is_err());
    }

    #[test]
    fn the_system_clock_is_recent() {
        assert!(SystemClock.now().0 > 1_577_836_800);
    }
}
