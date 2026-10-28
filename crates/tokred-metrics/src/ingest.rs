//! Incremental transcript ingestion.
//!
//! Each transcript is read from the byte offset saved in `ingest_state`. Only complete lines are
//! consumed: a trailing partial line (Claude Code is still writing it) is left for the next run,
//! so the saved offset always sits right after a `\n`. A file that shrank is read again from the
//! start. Re-reading is harmless because `usage` is deduplicated on `message_key`.

use serde::Serialize;
use tokred_core::FsError;
use tokred_core::usage::{IngestRepo, IngestState, UsageRecord, UsageRepo};

use crate::error::MetricsError;
use crate::transcript::{LineContext, parse_line};
use crate::util::len_u64;

/// A transcript file found under `<claude>/projects`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptFile {
    /// Path relative to `<claude>/projects`, separated by `/` on every OS
    /// (`<project>/<session>.jsonl` or `<project>/<session>/subagents/agent-*.jsonl`).
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// Modification time, unix seconds.
    pub mtime: i64,
}

/// Port: access to the transcript files.
pub trait TranscriptFs {
    /// Lists every `*.jsonl` file under the projects directory (empty if it does not exist).
    fn list(&self) -> Result<Vec<TranscriptFile>, FsError>;
    /// Reads the bytes of `path` (as returned by [`TranscriptFs::list`]) from `offset` to the end.
    fn read_from(&self, path: &str, offset: u64) -> Result<Vec<u8>, FsError>;
}

/// What one [`ingest`] run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct IngestSummary {
    /// Transcript files found.
    pub files_seen: u64,
    /// Files that had new bytes and were read.
    pub files_read: u64,
    /// Bytes of complete lines consumed.
    pub bytes_read: u64,
    /// Usage records extracted (before deduplication against the database).
    pub records_found: u64,
    /// Records that were new to the database.
    pub records_inserted: u64,
}

/// Project, session fallback and subagent flag derived from a root-relative transcript path.
fn context_of(path: &str) -> (String, String, bool) {
    let parts: Vec<&str> = path.split('/').collect();
    let project = if parts.len() > 1 { parts[0] } else { "" };
    let subagent_dir = parts.iter().position(|part| *part == "subagents");
    let file = parts.last().copied().unwrap_or_default();
    let stem = file.strip_suffix(".jsonl").unwrap_or(file);
    // Subagent transcripts belong to the session whose directory contains them.
    let session = match subagent_dir {
        Some(index) if index > 0 => parts[index - 1],
        _ => stem,
    };
    (
        project.to_owned(),
        session.to_owned(),
        subagent_dir.is_some(),
    )
}

/// Parses every complete line of `bytes`; returns the records and how many bytes were consumed
/// (up to and including the last `\n`).
fn parse_complete_lines(bytes: &[u8], path: &str) -> (Vec<UsageRecord>, usize) {
    let consumed = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let (project, session, is_subagent_file) = context_of(path);
    let context = LineContext {
        project: &project,
        fallback_session: &session,
        is_subagent_file,
    };
    let text = String::from_utf8_lossy(&bytes[..consumed]);
    let records = text
        .lines()
        .filter_map(|line| parse_line(line, &context))
        .collect();
    (records, consumed)
}

/// Ingests every new transcript line into `usage` and advances `ingest_state`.
///
/// Per file: resume at the saved offset (or 0 if the file is smaller than it), skip files with
/// nothing new, insert the records of the complete lines, then save the new offset. Malformed
/// lines are skipped. The offset is only saved after the records were stored, so a failure
/// between the two re-reads the lines instead of losing them.
pub fn ingest(
    fs: &dyn TranscriptFs,
    usage: &dyn UsageRepo,
    state: &dyn IngestRepo,
) -> Result<IngestSummary, MetricsError> {
    let files = fs.list()?;
    let mut summary = IngestSummary {
        files_seen: len_u64(files.len()),
        ..IngestSummary::default()
    };
    for file in &files {
        let saved = state.ingest_state(&file.path)?;
        let saved_offset = saved.as_ref().map(|saved| saved.offset);
        // A file smaller than the saved offset was truncated or rotated: start over.
        let offset = saved_offset
            .filter(|saved| *saved <= file.size)
            .unwrap_or(0);
        if file.size == offset && saved_offset == Some(offset) {
            continue;
        }
        let bytes = fs.read_from(&file.path, offset)?;
        let (records, consumed) = parse_complete_lines(&bytes, &file.path);
        let inserted = usage.insert_new(&records)?;
        state.save_ingest_state(&IngestState {
            file: file.path.clone(),
            offset: offset + len_u64(consumed),
            mtime: file.mtime,
        })?;
        summary.files_read += 1;
        summary.bytes_read += len_u64(consumed);
        summary.records_found += len_u64(records.len());
        summary.records_inserted += len_u64(inserted);
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use tokred_core::{StoreError, Timestamp};

    use super::*;

    #[derive(Default)]
    struct FakeFs {
        files: RefCell<BTreeMap<String, (Vec<u8>, i64)>>,
        fail_list: bool,
        fail_read: bool,
    }

    impl FakeFs {
        fn put(&self, path: &str, content: &str, mtime: i64) {
            self.files
                .borrow_mut()
                .insert(path.to_owned(), (content.as_bytes().to_vec(), mtime));
        }
    }

    impl TranscriptFs for FakeFs {
        fn list(&self) -> Result<Vec<TranscriptFile>, FsError> {
            if self.fail_list {
                return Err(FsError::new("/root", std::io::Error::other("list")));
            }
            Ok(self
                .files
                .borrow()
                .iter()
                .map(|(path, (bytes, mtime))| TranscriptFile {
                    path: path.clone(),
                    size: len_u64(bytes.len()),
                    mtime: *mtime,
                })
                .collect())
        }

        fn read_from(&self, path: &str, offset: u64) -> Result<Vec<u8>, FsError> {
            if self.fail_read {
                return Err(FsError::new(path, std::io::Error::other("read")));
            }
            let files = self.files.borrow();
            let (bytes, _) = &files[path];
            Ok(bytes[usize::try_from(offset).unwrap()..].to_vec())
        }
    }

    #[derive(Default)]
    struct FakeRepos {
        rows: RefCell<BTreeMap<String, UsageRecord>>,
        states: RefCell<BTreeMap<String, IngestState>>,
        fail_insert: bool,
        fail_state: bool,
    }

    impl UsageRepo for FakeRepos {
        fn insert_new(&self, records: &[UsageRecord]) -> Result<usize, StoreError> {
            if self.fail_insert {
                return Err(StoreError::backend(std::io::Error::other("insert")));
            }
            let mut rows = self.rows.borrow_mut();
            let before = rows.len();
            for record in records {
                rows.entry(record.message_key.clone())
                    .or_insert_with(|| record.clone());
            }
            Ok(rows.len() - before)
        }

        fn usage_between(
            &self,
            _: Timestamp,
            _: Timestamp,
        ) -> Result<Vec<UsageRecord>, StoreError> {
            Ok(self.rows.borrow().values().cloned().collect())
        }

        fn earliest_usage(&self) -> Result<Option<Timestamp>, StoreError> {
            Ok(self.rows.borrow().values().map(|r| r.ts).min())
        }
    }

    impl IngestRepo for FakeRepos {
        fn ingest_state(&self, file: &str) -> Result<Option<IngestState>, StoreError> {
            if self.fail_state {
                return Err(StoreError::backend(std::io::Error::other("state")));
            }
            Ok(self.states.borrow().get(file).cloned())
        }

        fn save_ingest_state(&self, state: &IngestState) -> Result<(), StoreError> {
            self.states
                .borrow_mut()
                .insert(state.file.clone(), state.clone());
            Ok(())
        }
    }

    fn line(id: &str, request: &str, output: u64) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"s1","timestamp":"2026-03-01T10:00:00Z","requestId":"{request}","message":{{"id":"{id}","model":"claude-sonnet-4-20250514","usage":{{"input_tokens":1,"output_tokens":{output},"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}}}}"#
        )
    }

    #[test]
    fn contexts_follow_the_directory_layout() {
        assert_eq!(
            context_of("-p/abc.jsonl"),
            ("-p".to_owned(), "abc".to_owned(), false)
        );
        assert_eq!(
            context_of("-p/abc/subagents/agent-7.jsonl"),
            ("-p".to_owned(), "abc".to_owned(), true)
        );
        assert_eq!(
            context_of("loose.jsonl"),
            (String::new(), "loose".to_owned(), false)
        );
        assert_eq!(
            context_of("subagents/agent-7.jsonl"),
            ("subagents".to_owned(), "agent-7".to_owned(), true)
        );
    }

    #[test]
    fn ingests_then_resumes_without_duplicates() {
        let fs = FakeFs::default();
        let repos = FakeRepos::default();
        let first = format!("{}\n{}\n", line("m1", "r1", 5), line("m2", "r2", 6));
        fs.put("-p/s1.jsonl", &first, 100);

        let summary = ingest(&fs, &repos, &repos).unwrap();
        assert_eq!(
            summary,
            IngestSummary {
                files_seen: 1,
                files_read: 1,
                bytes_read: len_u64(first.len()),
                records_found: 2,
                records_inserted: 2
            }
        );
        let state = repos.ingest_state("-p/s1.jsonl").unwrap().unwrap();
        assert_eq!(state.offset, len_u64(first.len()));
        assert_eq!(state.mtime, 100);

        // Nothing new: the file is skipped.
        let again = ingest(&fs, &repos, &repos).unwrap();
        assert_eq!((again.files_seen, again.files_read), (1, 0));

        // Appended lines, including a repeat of m2 (streaming): only m3 is new.
        let appended = format!("{first}{}\n{}\n", line("m2", "r2", 6), line("m3", "r3", 7));
        fs.put("-p/s1.jsonl", &appended, 200);
        let third = ingest(&fs, &repos, &repos).unwrap();
        assert_eq!((third.records_found, third.records_inserted), (2, 1));
        assert_eq!(repos.rows.borrow().len(), 3);
        let all = repos
            .usage_between(Timestamp(0), Timestamp(i64::MAX))
            .unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(
            repos.earliest_usage().unwrap(),
            Some(Timestamp(1_772_359_200))
        );
    }

    #[test]
    fn a_partial_last_line_waits_for_its_newline() {
        let fs = FakeFs::default();
        let repos = FakeRepos::default();
        let complete = format!("{}\n", line("m1", "r1", 5));
        let partial = line("m2", "r2", 6);
        fs.put("-p/s1.jsonl", &format!("{complete}{}", &partial[..40]), 1);

        let first = ingest(&fs, &repos, &repos).unwrap();
        assert_eq!(first.records_inserted, 1);
        assert_eq!(
            repos.ingest_state("-p/s1.jsonl").unwrap().unwrap().offset,
            len_u64(complete.len()),
            "offset stops after the last complete line"
        );

        fs.put("-p/s1.jsonl", &format!("{complete}{partial}\n"), 2);
        let second = ingest(&fs, &repos, &repos).unwrap();
        assert_eq!(second.records_inserted, 1);
        assert_eq!(repos.rows.borrow().len(), 2);
    }

    #[test]
    fn only_a_partial_line_consumes_nothing() {
        let fs = FakeFs::default();
        let repos = FakeRepos::default();
        fs.put("-p/s1.jsonl", "{\"type\":\"assis", 1);
        let summary = ingest(&fs, &repos, &repos).unwrap();
        assert_eq!((summary.files_read, summary.bytes_read), (1, 0));
        assert_eq!(
            repos.ingest_state("-p/s1.jsonl").unwrap().unwrap().offset,
            0
        );
    }

    #[test]
    fn a_truncated_file_is_read_again_from_the_start() {
        let fs = FakeFs::default();
        let repos = FakeRepos::default();
        let big = format!("{}\n{}\n", line("m1", "r1", 5), line("m2", "r2", 6));
        fs.put("-p/s1.jsonl", &big, 1);
        ingest(&fs, &repos, &repos).unwrap();

        // Rotated: a shorter file with a new message.
        let small = format!("{}\n", line("m9", "r9", 1));
        assert!(small.len() < big.len());
        fs.put("-p/s1.jsonl", &small, 2);
        let summary = ingest(&fs, &repos, &repos).unwrap();
        assert_eq!((summary.records_found, summary.records_inserted), (1, 1));
        assert_eq!(
            repos.ingest_state("-p/s1.jsonl").unwrap().unwrap().offset,
            len_u64(small.len())
        );
        assert_eq!(repos.rows.borrow().len(), 3);
    }

    #[test]
    fn a_file_emptied_and_refilled_is_not_resumed_at_the_old_offset() {
        let fs = FakeFs::default();
        let repos = FakeRepos::default();
        let big = format!("{}\n{}\n", line("m1", "r1", 5), line("m2", "r2", 6));
        fs.put("-p/s1.jsonl", &big, 1);
        ingest(&fs, &repos, &repos).unwrap();

        fs.put("-p/s1.jsonl", "", 2);
        ingest(&fs, &repos, &repos).unwrap();
        assert_eq!(
            repos.ingest_state("-p/s1.jsonl").unwrap().unwrap().offset,
            0
        );

        fs.put("-p/s1.jsonl", &format!("{}\n", line("m7", "r7", 1)), 3);
        let summary = ingest(&fs, &repos, &repos).unwrap();
        assert_eq!(summary.records_inserted, 1);
    }

    #[test]
    fn subagent_files_are_flagged_and_malformed_lines_skipped() {
        let fs = FakeFs::default();
        let repos = FakeRepos::default();
        let content = format!(
            "garbage\r\n{}\r\n\n{{\"type\":\"user\"}}\n",
            line("a1", "r1", 3).replace("\"sessionId\":\"s1\",", "")
        );
        fs.put("-p/s1/subagents/agent-1.jsonl", &content, 1);
        let summary = ingest(&fs, &repos, &repos).unwrap();
        assert_eq!(summary.records_inserted, 1);
        let rows = repos.rows.borrow();
        let row = rows.values().next().unwrap();
        assert!(row.is_subagent);
        assert_eq!(
            row.session_id, "s1",
            "falls back to the owning session directory"
        );
        assert_eq!(row.project, "-p");
    }

    #[test]
    fn failures_are_reported_and_state_is_not_advanced() {
        let fs = FakeFs::default();
        fs.put("-p/s1.jsonl", &format!("{}\n", line("m1", "r1", 5)), 1);

        let repos = FakeRepos::default();
        let broken_list = FakeFs {
            fail_list: true,
            ..FakeFs::default()
        };
        assert!(matches!(
            ingest(&broken_list, &repos, &repos),
            Err(MetricsError::Fs(_))
        ));

        let broken_read = FakeFs {
            fail_read: true,
            ..FakeFs::default()
        };
        broken_read.put("-p/s1.jsonl", "x\n", 1);
        assert!(matches!(
            ingest(&broken_read, &repos, &repos),
            Err(MetricsError::Fs(_))
        ));

        let failing_insert = FakeRepos {
            fail_insert: true,
            ..FakeRepos::default()
        };
        let err = ingest(&fs, &failing_insert, &failing_insert).unwrap_err();
        assert!(matches!(err, MetricsError::Store(_)));
        assert!(err.to_string().contains("insert"));
        assert!(failing_insert.states.borrow().is_empty());

        let failing_state = FakeRepos {
            fail_state: true,
            ..FakeRepos::default()
        };
        assert!(matches!(
            ingest(&fs, &failing_state, &failing_state),
            Err(MetricsError::Store(_))
        ));
    }
}
