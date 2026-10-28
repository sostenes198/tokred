//! `std` + `walkdir` adapter of the [`TranscriptFs`] port.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use tokred_core::FsError;
use walkdir::{DirEntry, WalkDir};

use crate::ingest::{TranscriptFile, TranscriptFs};

/// Reads transcripts from a real directory (`<claude>/projects`).
#[derive(Debug, Clone)]
pub struct StdTranscriptFs {
    root: PathBuf,
}

impl StdTranscriptFs {
    /// Adapter rooted at the `projects` directory of the Claude Code folder.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

/// Size, mtime and root-relative `/`-separated path of a `.jsonl` file; `None` for anything else
/// or for entries whose metadata cannot be read.
fn describe(root: &Path, entry: &DirEntry) -> Option<TranscriptFile> {
    if !entry.file_type().is_file() || entry.path().extension()? != "jsonl" {
        return None;
    }
    let meta = entry.metadata().ok()?;
    let relative = entry.path().strip_prefix(root).ok()?;
    let path = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    let mtime = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(0);
    Some(TranscriptFile {
        path,
        size: meta.len(),
        mtime,
    })
}

fn read_range(path: &Path, offset: u64) -> std::io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

impl TranscriptFs for StdTranscriptFs {
    fn list(&self) -> Result<Vec<TranscriptFile>, FsError> {
        // A missing `projects` directory simply means there is nothing to ingest yet, and entries
        // that cannot be read (permissions, races with deletion) are skipped rather than fatal.
        let mut files: Vec<_> = WalkDir::new(&self.root)
            .into_iter()
            .filter_map(Result::ok)
            .filter_map(|entry| describe(&self.root, &entry))
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    fn read_from(&self, path: &str, offset: u64) -> Result<Vec<u8>, FsError> {
        let full = self.root.join(path);
        read_range(&full, offset).map_err(|source| FsError::new(full.clone(), source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_jsonl_files_with_relative_slash_paths() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("proj/sess/subagents");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(dir.path().join("proj/sess.jsonl"), "abc\n").unwrap();
        std::fs::write(nested.join("agent-1.jsonl"), "defg\n").unwrap();
        std::fs::write(dir.path().join("proj/notes.txt"), "x").unwrap();
        std::fs::write(dir.path().join("proj/noext"), "x").unwrap();

        let files = StdTranscriptFs::new(dir.path()).list().unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            ["proj/sess.jsonl", "proj/sess/subagents/agent-1.jsonl"]
        );
        assert_eq!(files[0].size, 4);
        assert_eq!(files[1].size, 5);
        assert!(files[0].mtime > 1_600_000_000);
    }

    #[test]
    fn a_missing_root_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let fs = StdTranscriptFs::new(dir.path().join("absent"));
        assert!(fs.list().unwrap().is_empty());
    }

    #[test]
    fn reads_from_an_offset() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jsonl"), "0123456789").unwrap();
        let fs = StdTranscriptFs::new(dir.path());
        assert_eq!(fs.read_from("a.jsonl", 0).unwrap(), b"0123456789");
        assert_eq!(fs.read_from("a.jsonl", 4).unwrap(), b"456789");
        assert!(fs.read_from("a.jsonl", 99).unwrap().is_empty());
    }

    #[test]
    fn reading_a_missing_file_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let err = StdTranscriptFs::new(dir.path())
            .read_from("gone.jsonl", 0)
            .unwrap_err();
        assert!(err.to_string().contains("gone.jsonl"));
    }
}
