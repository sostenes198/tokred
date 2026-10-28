#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The `std` adapters over real temporary directories.

use std::fs::{self, File, FileTimes};
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use tokred_audit::fs::{StdDirLister, TranscriptScanner};
use tokred_audit::{DirLister, McpUsageSource, UsageError};
use tokred_core::Timestamp;

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn set_mtime(path: &Path, unix: i64) {
    let file = File::options().write(true).open(path).unwrap();
    let time = UNIX_EPOCH + Duration::from_secs(u64::try_from(unix).unwrap());
    file.set_times(FileTimes::new().set_modified(time)).unwrap();
}

fn call(id: &str, tool: &str, ts: &str) -> String {
    format!(
        r#"{{"type":"assistant","timestamp":"{ts}","message":{{"content":[{{"type":"tool_use","id":"{id}","name":"{tool}","input":{{}}}}]}}}}"#
    )
}

/// 2026-01-02T03:04:05Z
const T0: i64 = 1_767_323_045;

#[test]
fn counts_calls_per_server_inside_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let lines = [
        call("1", "mcp__github__create_issue", "2026-01-02T03:04:05Z"),
        call("2", "mcp__github__list_prs", "2026-01-03T00:00:00.500Z"),
        call("3", "mcp__slack__post", "2026-01-02T03:04:04Z"),
        call("4", "Bash", "2026-01-05T00:00:00Z"),
        "this line mentions mcp__github__x but is not json".to_owned(),
        "{\"type\":\"user\",\"message\":{\"content\":\"mcp__github__x in a prompt\"}}".to_owned(),
        "{}".to_owned(),
    ];
    write(&root.join("proj-a/s1.jsonl"), &lines.join("\n"));
    let scan = TranscriptScanner::new(root)
        .usage_since(Timestamp(T0))
        .unwrap();
    assert_eq!(scan.files_scanned, 1);
    // Boundary: a call exactly at `since` counts, one second earlier does not.
    assert_eq!(scan.servers["github"].calls, 2);
    assert_eq!(
        scan.servers["github"].last_seen,
        Some(Timestamp(T0 + 86_400 - 3 * 3600 - 4 * 60 - 5))
    );
    assert!(!scan.servers.contains_key("slack"));
    assert_eq!(scan.servers.len(), 1);
}

#[test]
fn streamed_duplicates_are_counted_once_and_subagents_are_included() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let dup = call("same", "mcp__a__t", "2026-02-01T00:00:00Z");
    write(&root.join("p/s.jsonl"), &format!("{dup}\n{dup}\n"));
    write(
        &root.join("p/s/subagents/agent-1.jsonl"),
        &format!(
            "{dup}\n{}\n",
            call("other", "mcp__a__t", "2026-02-02T00:00:00Z")
        ),
    );
    write(
        &root.join("p/s/tool-results/big.jsonl"),
        &call("x", "mcp__a__t", "2026-02-03T00:00:00Z"),
    );
    write(
        &root.join("p/notes.txt"),
        &call("y", "mcp__a__t", "2026-02-03T00:00:00Z"),
    );
    let scan = TranscriptScanner::new(root)
        .usage_since(Timestamp(0))
        .unwrap();
    assert_eq!(scan.files_scanned, 2);
    assert_eq!(scan.servers["a"].calls, 2);
}

#[test]
fn files_not_modified_inside_the_window_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let old = root.join("p/old.jsonl");
    let edge = root.join("p/edge.jsonl");
    write(&old, &call("1", "mcp__a__t", "2026-01-02T03:04:05Z"));
    write(&edge, &call("2", "mcp__b__t", "2026-01-02T03:04:05Z"));
    set_mtime(&old, 1_000);
    set_mtime(&edge, T0);
    let scan = TranscriptScanner::new(root)
        .usage_since(Timestamp(T0))
        .unwrap();
    assert_eq!(scan.files_scanned, 1);
    assert!(scan.servers.contains_key("b") && !scan.servers.contains_key("a"));
}

#[test]
fn lines_without_a_usable_timestamp_use_the_file_modification_time() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let no_ts = r#"{"message":{"content":[{"type":"tool_use","id":"n1","name":"mcp__a__t"}]}}"#;
    let bad_ts = r#"{"timestamp":"yesterday","message":{"content":[{"type":"tool_use","id":"n2","name":"mcp__a__t"}]}}"#;
    let file = root.join("p/s.jsonl");
    write(&file, &format!("{no_ts}\n{bad_ts}\n"));
    set_mtime(&file, T0 + 100);
    let scan = TranscriptScanner::new(root)
        .usage_since(Timestamp(T0))
        .unwrap();
    assert_eq!(scan.servers["a"].calls, 2);
    assert_eq!(scan.servers["a"].last_seen, Some(Timestamp(T0 + 100)));
}

#[test]
fn invalid_utf8_does_not_abandon_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("p/s.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = vec![0xff, 0xfe, b'\n'];
    bytes.extend(call("1", "mcp__a__t", "2026-03-01T00:00:00Z").into_bytes());
    fs::write(&path, bytes).unwrap();
    let scan = TranscriptScanner::new(dir.path())
        .usage_since(Timestamp(0))
        .unwrap();
    assert_eq!(scan.servers["a"].calls, 1);
}

#[test]
fn missing_root_is_empty_and_a_file_root_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let scan = TranscriptScanner::new(dir.path().join("absent"))
        .usage_since(Timestamp(0))
        .unwrap();
    assert_eq!(scan.files_scanned, 0);
    assert!(scan.servers.is_empty());
    let file = dir.path().join("file");
    fs::write(&file, "x").unwrap();
    let err = TranscriptScanner::new(&file)
        .usage_since(Timestamp(0))
        .unwrap_err();
    assert!(matches!(err, UsageError::Unavailable { .. }));
    assert!(err.to_string().contains("not a directory"));
}

#[test]
fn a_transcript_that_cannot_be_read_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    // A directory named like a transcript is not a file, so it is never scanned.
    fs::create_dir_all(dir.path().join("p/weird.jsonl/inner")).unwrap();
    write(
        &dir.path().join("p/ok.jsonl"),
        &call("1", "mcp__a__t", "2026-03-01T00:00:00Z"),
    );
    let scan = TranscriptScanner::new(dir.path())
        .usage_since(Timestamp(0))
        .unwrap();
    assert_eq!(scan.files_scanned, 1);
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_is_not_a_readable_transcript() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("p")).unwrap();
    std::os::unix::fs::symlink(dir.path().join("nowhere"), dir.path().join("p/dead.jsonl"))
        .unwrap();
    let scan = TranscriptScanner::new(dir.path())
        .usage_since(Timestamp(0))
        .unwrap();
    assert_eq!(scan.files_scanned, 0);
}

#[test]
fn lists_directories_sorted_and_tolerates_missing_ones() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("b.md"), "");
    write(&dir.path().join("a/inner.md"), "");
    let entries = StdDirLister.list_dir(dir.path()).unwrap();
    let shape: Vec<(String, bool)> = entries
        .iter()
        .map(|e| {
            (
                e.path.file_name().unwrap().to_string_lossy().into_owned(),
                e.is_dir,
            )
        })
        .collect();
    assert_eq!(shape, [("a".to_owned(), true), ("b.md".to_owned(), false)]);
    assert!(
        StdDirLister
            .list_dir(&dir.path().join("none"))
            .unwrap()
            .is_empty()
    );
    let err = StdDirLister.list_dir(&dir.path().join("b.md")).unwrap_err();
    assert!(err.to_string().contains("b.md"));
}
