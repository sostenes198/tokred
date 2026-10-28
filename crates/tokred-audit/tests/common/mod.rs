#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]
//! In-memory ports shared by the integration tests.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use tokred_audit::{
    AuditReport, AuditRequest, DirEntry, DirLister, McpUsageSource, Ports, UsageError, UsageScan,
};
use tokred_core::{Clock, FileSystem, FsError, Timestamp};

/// Files in memory; paths are written with `/` separators.
#[derive(Default)]
pub struct MemFs {
    files: BTreeMap<PathBuf, String>,
    unreadable: BTreeSet<PathBuf>,
    unlistable: BTreeSet<PathBuf>,
}

impl MemFs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn file(mut self, path: &str, content: &str) -> Self {
        self.files.insert(PathBuf::from(path), content.to_owned());
        self
    }

    /// A file that exists but fails to read (permissions, invalid UTF-8...).
    pub fn unreadable(mut self, path: &str) -> Self {
        self.unreadable.insert(PathBuf::from(path));
        self
    }

    /// A directory whose listing fails.
    pub fn unlistable(mut self, path: &str) -> Self {
        self.unlistable.insert(PathBuf::from(path));
        self
    }
}

impl FileSystem for MemFs {
    fn read_to_string(&self, path: &Path) -> Result<Option<String>, FsError> {
        if self.unreadable.contains(path) {
            return Err(FsError::new(path, io::Error::other("denied")));
        }
        Ok(self.files.get(path).cloned())
    }
    fn write_atomic(&self, path: &Path, _contents: &[u8]) -> Result<(), FsError> {
        Err(FsError::new(path, io::Error::other("read-only")))
    }
    fn remove_file(&self, _path: &Path) -> Result<bool, FsError> {
        Ok(false)
    }
    fn remove_dir_all(&self, _path: &Path) -> Result<bool, FsError> {
        Ok(false)
    }
    fn create_dir_all(&self, _path: &Path) -> Result<(), FsError> {
        Ok(())
    }
    fn exists(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }
    fn is_dir(&self, _path: &Path) -> bool {
        false
    }
}

impl DirLister for MemFs {
    fn list_dir(&self, dir: &Path) -> Result<Vec<DirEntry>, FsError> {
        if self.unlistable.contains(dir) {
            return Err(FsError::new(dir, io::Error::other("denied")));
        }
        let mut children: BTreeMap<PathBuf, bool> = BTreeMap::new();
        for file in self.files.keys() {
            let Ok(rest) = file.strip_prefix(dir) else {
                continue;
            };
            let mut parts = rest.components();
            let Some(first) = parts.next() else {
                continue;
            };
            let is_dir = parts.next().is_some();
            let entry = children.entry(dir.join(first)).or_insert(false);
            *entry |= is_dir;
        }
        Ok(children
            .into_iter()
            .map(|(path, is_dir)| DirEntry { path, is_dir })
            .collect())
    }
}

/// A fixed clock.
pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0)
    }
}

/// A usage source that returns a canned answer and records the requested window start.
pub struct FakeUsage {
    pub result: Result<UsageScan, String>,
    pub requested: std::cell::Cell<Option<i64>>,
}

impl FakeUsage {
    pub fn ok(entries: &[(&str, u64, Option<i64>)]) -> Self {
        let servers = entries
            .iter()
            .map(|(name, calls, last)| {
                (
                    (*name).to_owned(),
                    tokred_audit::McpUsage {
                        calls: *calls,
                        last_seen: last.map(Timestamp),
                    },
                )
            })
            .collect();
        Self {
            result: Ok(UsageScan {
                servers,
                files_scanned: 3,
            }),
            requested: std::cell::Cell::new(None),
        }
    }

    pub fn no_files() -> Self {
        Self {
            result: Ok(UsageScan::default()),
            requested: std::cell::Cell::new(None),
        }
    }

    pub fn failing() -> Self {
        Self {
            result: Err("boom".to_owned()),
            requested: std::cell::Cell::new(None),
        }
    }
}

impl McpUsageSource for FakeUsage {
    fn usage_since(&self, since: Timestamp) -> Result<UsageScan, UsageError> {
        self.requested.set(Some(since.0));
        self.result
            .clone()
            .map_err(|reason| UsageError::Unavailable {
                path: PathBuf::from("/h/.claude/projects"),
                reason,
            })
    }
}

pub const NOW: i64 = 1_800_000_000;

pub fn run(fs: &MemFs, usage: &FakeUsage, request: &AuditRequest) -> AuditReport {
    let clock = FixedClock(NOW);
    let ports = Ports {
        fs,
        dirs: fs,
        usage,
        clock: &clock,
    };
    tokred_audit::audit(&ports, request)
}

/// Request for the standard layout: claude dir `/h/.claude`, home `/h`, project `/h/proj`.
pub fn request(with_project: bool) -> AuditRequest {
    AuditRequest::new(
        "/h/.claude",
        Some(PathBuf::from("/h")),
        with_project.then(|| PathBuf::from("/h/proj")),
    )
}

pub fn item<'a>(report: &'a AuditReport, file: &str) -> &'a tokred_audit::AuditItem {
    report
        .items
        .iter()
        .find(|i| i.path == Path::new(file))
        .unwrap_or_else(|| panic!("no item for {file}: {:?}", report.items))
}

pub fn has_warning(report: &AuditReport, needle: &str) -> bool {
    report.warnings.iter().any(|w| w.contains(needle))
}
