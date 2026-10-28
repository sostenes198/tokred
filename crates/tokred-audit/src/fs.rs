//! `std` adapters: directory listing and the transcript scanner behind [`McpUsageSource`].

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokred_core::{FsError, Timestamp};
use walkdir::WalkDir;

use crate::ports::{DirEntry, DirLister, McpUsage, McpUsageSource, UsageError, UsageScan};

/// The real file system as a [`DirLister`].
#[derive(Debug, Clone, Copy, Default)]
pub struct StdDirLister;

impl DirLister for StdDirLister {
    fn list_dir(&self, dir: &Path) -> Result<Vec<DirEntry>, FsError> {
        let read = match std::fs::read_dir(dir) {
            Ok(read) => read,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(FsError::new(dir, err)),
        };
        let mut entries: Vec<DirEntry> = read
            .filter_map(Result::ok)
            .map(|entry| DirEntry {
                is_dir: entry.path().is_dir(),
                path: entry.path(),
            })
            .collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }
}

/// Scans `<claude>/projects/**/*.jsonl` for MCP `tool_use` calls.
///
/// Efficiency rules: files not modified inside the window are skipped; each file is read line by
/// line (invalid UTF-8 is tolerated); only lines containing `mcp__` are JSON-parsed; malformed
/// lines are ignored; `tool_use` ids are de-duplicated because streaming repeats lines.
#[derive(Debug, Clone)]
pub struct TranscriptScanner {
    projects_dir: PathBuf,
}

impl TranscriptScanner {
    /// Creates a scanner over `projects_dir` (normally `<claude>/projects`).
    #[must_use]
    pub fn new(projects_dir: impl Into<PathBuf>) -> Self {
        Self {
            projects_dir: projects_dir.into(),
        }
    }
}

/// One MCP `tool_use` found in a transcript line.
#[derive(Debug, PartialEq, Eq)]
struct Call {
    server: String,
    id: Option<String>,
    at: Option<i64>,
}

/// Server segment of `mcp__<server>__<tool>`.
fn server_of(tool_name: &str) -> Option<&str> {
    let (server, _tool) = tool_name.strip_prefix("mcp__")?.split_once("__")?;
    (!server.is_empty()).then_some(server)
}

/// MCP calls of one transcript line (empty for lines that are not assistant tool calls).
fn calls_in_line(raw: &[u8]) -> Vec<Call> {
    let line = String::from_utf8_lossy(raw);
    if !line.contains("mcp__") {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<Value>(&line) else {
        return Vec::new();
    };
    let at = value
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|text| OffsetDateTime::parse(text, &Rfc3339).ok())
        .map(OffsetDateTime::unix_timestamp);
    let blocks = value
        .pointer("/message/content")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    blocks
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        .filter_map(|block| {
            let server = server_of(block.get("name")?.as_str()?)?;
            Some(Call {
                server: server.to_owned(),
                id: block.get("id").and_then(Value::as_str).map(str::to_owned),
                at,
            })
        })
        .collect()
}

fn mtime_secs(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let secs = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(i64::try_from(secs).unwrap_or(i64::MAX))
}

/// Reads one transcript; `None` when it is outside the window or cannot be opened.
fn scan_file(
    path: &Path,
    since: i64,
    seen: &mut HashSet<String>,
    servers: &mut BTreeMap<String, McpUsage>,
) -> Option<()> {
    let mtime = mtime_secs(path);
    if mtime.is_some_and(|m| m < since) {
        return None;
    }
    let mut reader = BufReader::new(File::open(path).ok()?);
    let mut buf = Vec::new();
    while reader.read_until(b'\n', &mut buf).ok()? > 0 {
        for call in calls_in_line(&buf) {
            let at = call.at.or(mtime);
            let fresh = call.id.is_none_or(|id| seen.insert(id));
            if fresh && at.is_none_or(|t| t >= since) {
                let usage = servers.entry(call.server).or_default();
                usage.calls += 1;
                usage.last_seen = usage.last_seen.max(at.map(Timestamp));
            }
        }
        buf.clear();
    }
    Some(())
}

impl McpUsageSource for TranscriptScanner {
    fn usage_since(&self, since: Timestamp) -> Result<UsageScan, UsageError> {
        let root = &self.projects_dir;
        if !root.exists() {
            return Ok(UsageScan::default());
        }
        if !root.is_dir() {
            return Err(UsageError::Unavailable {
                path: root.clone(),
                reason: "not a directory".to_owned(),
            });
        }
        let mut scan = UsageScan::default();
        let mut seen = HashSet::new();
        let files = WalkDir::new(root)
            .sort_by_file_name()
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "jsonl"))
            .filter(|e| {
                !e.path()
                    .components()
                    .any(|c| c.as_os_str() == "tool-results")
            });
        for entry in files {
            if scan_file(entry.path(), since.0, &mut seen, &mut scan.servers).is_some() {
                scan.files_scanned += 1;
            }
        }
        Ok(scan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_segment_is_parsed() {
        assert_eq!(server_of("mcp__github__create_issue"), Some("github"));
        assert_eq!(server_of("mcp__plugin_a_b__t__x"), Some("plugin_a_b"));
        assert_eq!(server_of("mcp____tool"), None);
        assert_eq!(server_of("mcp__noseparator"), None);
        assert_eq!(server_of("Bash"), None);
    }

    #[test]
    fn lines_without_mcp_or_with_bad_json_yield_nothing() {
        assert!(calls_in_line(b"{\"a\":1}").is_empty());
        assert!(calls_in_line(b"not json mcp__x__y").is_empty());
        assert!(calls_in_line(b"{\"mcp__\":1}").is_empty());
    }

    #[test]
    fn extracts_only_tool_use_blocks() {
        let line = br#"{"timestamp":"2026-01-02T03:04:05.123Z","message":{"content":[
            {"type":"text","text":"mcp__a__b"},
            {"type":"tool_use","id":"t1","name":"mcp__a__b"},
            {"type":"tool_use","id":"t2","name":"Bash"},
            {"type":"tool_use","name":"mcp__c__d"}]}}"#;
        let calls = calls_in_line(line);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].server, "a");
        assert_eq!(calls[0].id.as_deref(), Some("t1"));
        assert_eq!(calls[0].at, Some(1_767_323_045));
        assert_eq!(calls[1].server, "c");
        assert_eq!(calls[1].id, None);
    }
}
