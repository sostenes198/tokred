//! The name of a session in the dashboard: what Claude Code itself shows for it.
//!
//! Claude Code writes the title (the one it generated, or the one you gave the session) into the
//! session's transcript, `<claude dir>/projects/<project>/<session id>.jsonl`, and writes it again
//! as the conversation goes on. The end of the file is enough to read it; the start is tried when
//! the end has none. A transcript can be tens of megabytes, so a result is kept until the file
//! changes.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use tokred_core::dashboard::{SessionLabel, session_label};

/// How much of the end of a transcript is read.
const TAIL_BYTES: u64 = 512 * 1024;
/// How much of the start is read when the end has no title.
const HEAD_BYTES: u64 = 128 * 1024;
/// The longest session id looked up (Claude Code's are 36 characters).
const MAX_ID_CHARS: usize = 64;

/// A label and the state of the file it was read from.
struct Cached {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: u64,
    label: SessionLabel,
}

/// Finds and reads the labels of sessions, remembering them until the transcript changes.
pub struct Labels {
    projects: Option<PathBuf>,
    cache: Mutex<HashMap<String, Cached>>,
}

impl Labels {
    /// Labels read from the transcripts under `claude_dir` (none are found without it).
    #[must_use]
    pub fn new(claude_dir: Option<&Path>) -> Self {
        Self {
            projects: claude_dir.map(|dir| dir.join("projects")),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The label of session `id`; empty when its transcript is not found or unreadable.
    pub fn label(&self, id: &str) -> SessionLabel {
        let Some((path, modified, len)) = self.located(id) else {
            return SessionLabel::default();
        };
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(hit) = cache.get(id)
            && hit.path == path
            && hit.modified == modified
            && hit.len == len
        {
            return hit.label.clone();
        }
        let label = read_label(&path, len);
        cache.insert(
            id.to_owned(),
            Cached {
                path,
                modified,
                len,
                label: label.clone(),
            },
        );
        label
    }

    /// The transcript of session `id`, its modification time and its size; `None` when it is not
    /// there or cannot be inspected.
    fn located(&self, id: &str) -> Option<(PathBuf, Option<SystemTime>, u64)> {
        let path = self.transcript(id)?;
        let meta = std::fs::metadata(&path).ok()?;
        Some((path, meta.modified().ok(), meta.len()))
    }

    /// The transcript of session `id` in whichever project folder holds it. The id comes from a
    /// request, so only the characters of a real id are accepted before it joins a path.
    fn transcript(&self, id: &str) -> Option<PathBuf> {
        let safe = !id.is_empty()
            && id.chars().count() <= MAX_ID_CHARS
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        let projects = self.projects.as_ref().filter(|_| safe)?;
        std::fs::read_dir(projects)
            .ok()?
            .flatten()
            .map(|entry| entry.path().join(format!("{id}.jsonl")))
            .find(|path| path.is_file())
    }
}

/// The label of a transcript of `len` bytes: from its end, and from its start for what the end
/// does not say.
fn read_label(path: &Path, len: u64) -> SessionLabel {
    let tail_from = len.saturating_sub(TAIL_BYTES);
    let mut label = read_part(path, tail_from, TAIL_BYTES)
        .map_or_else(SessionLabel::default, |text| session_label(&text));
    if tail_from > 0 && (label.title.is_none() || label.project.is_none()) {
        let head = read_part(path, 0, HEAD_BYTES)
            .map(|text| session_label(&text))
            .unwrap_or_default();
        label.title = label.title.or(head.title);
        label.project = label.project.or(head.project);
    }
    label
}

/// `count` bytes of the file from `offset`, as text (a cut in the middle of a character is
/// replaced, not an error).
fn read_part(path: &Path, offset: u64, count: u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut bytes = Vec::new();
    file.take(count).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> (tempfile::TempDir, Labels) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("projects/-Users-x-tokred")).unwrap();
        std::fs::create_dir_all(dir.path().join("projects/-Users-x-other")).unwrap();
        let labels = Labels::new(Some(dir.path()));
        (dir, labels)
    }

    fn write(dir: &tempfile::TempDir, project: &str, id: &str, text: &str) -> PathBuf {
        let path = dir
            .path()
            .join("projects")
            .join(project)
            .join(format!("{id}.jsonl"));
        std::fs::write(&path, text).unwrap();
        path
    }

    const TITLE: &str =
        r#"{"type":"ai-title","aiTitle":"Tokred spec implementation","sessionId":"abc-1"}"#;
    const CWD: &str = r#"{"type":"user","cwd":"/Users/x/Repos/tokred","sessionId":"abc-1"}"#;

    #[test]
    fn the_title_and_project_come_from_the_transcript_in_any_project_folder() {
        let (dir, labels) = labels();
        write(
            &dir,
            "-Users-x-other",
            "abc-1",
            &format!("{CWD}\n{TITLE}\n"),
        );
        let label = labels.label("abc-1");
        assert_eq!(label.title.as_deref(), Some("Tokred spec implementation"));
        assert_eq!(label.project.as_deref(), Some("tokred"));
        assert_eq!(labels.label("unknown-session"), SessionLabel::default());
    }

    #[test]
    fn a_label_is_kept_until_the_file_changes() {
        let (dir, labels) = labels();
        let path = write(&dir, "-Users-x-tokred", "abc-1", &format!("{TITLE}\n"));
        assert_eq!(
            labels.label("abc-1").title.as_deref(),
            Some("Tokred spec implementation")
        );
        // Same size and time: the answer is the remembered one, whatever the file holds.
        let kept = std::fs::metadata(&path).unwrap().modified().unwrap();
        let swapped = TITLE.replace("Tokred spec implementation", "Another title entirely....");
        assert_eq!(swapped.len(), TITLE.len());
        std::fs::write(&path, format!("{swapped}\n")).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(kept)
            .unwrap();
        assert_eq!(
            labels.label("abc-1").title.as_deref(),
            Some("Tokred spec implementation")
        );
        // A longer file is read again.
        std::fs::write(&path, format!("{swapped}\n{CWD}\n")).unwrap();
        let label = labels.label("abc-1");
        assert_eq!(label.title.as_deref(), Some("Another title entirely...."));
        assert_eq!(label.project.as_deref(), Some("tokred"));
    }

    #[test]
    fn only_the_characters_of_a_real_id_reach_the_file_system() {
        let (dir, labels) = labels();
        write(&dir, "-Users-x-tokred", "abc-1", &format!("{TITLE}\n"));
        for bad in [
            "",
            "../abc-1",
            "a/b",
            "a\\b",
            "abc 1",
            "é",
            &"x".repeat(MAX_ID_CHARS + 1),
        ] {
            assert_eq!(labels.label(bad), SessionLabel::default(), "{bad:?}");
        }
        assert!(labels.label("abc-1").title.is_some());
    }

    #[test]
    fn a_long_transcript_is_read_from_its_end_and_its_start() {
        let (dir, labels) = labels();
        let filler = format!("{}\n", r#"{"type":"user","text":"padding"}"#).repeat(30_000);
        assert!(filler.len() as u64 > TAIL_BYTES);
        // The title is only at the start, the folder only at the end.
        write(
            &dir,
            "-Users-x-tokred",
            "head",
            &format!("{TITLE}\n{filler}{CWD}\n"),
        );
        let label = labels.label("head");
        assert_eq!(label.title.as_deref(), Some("Tokred spec implementation"));
        assert_eq!(label.project.as_deref(), Some("tokred"));
        // Both at the end: the start is not needed.
        write(
            &dir,
            "-Users-x-tokred",
            "tail",
            &format!("{filler}{CWD}\n{TITLE}\n"),
        );
        let label = labels.label("tail");
        assert_eq!(label.title.as_deref(), Some("Tokred spec implementation"));
        assert_eq!(label.project.as_deref(), Some("tokred"));
        // Neither anywhere: nothing, without failing.
        write(&dir, "-Users-x-tokred", "none", &filler);
        assert_eq!(labels.label("none"), SessionLabel::default());
    }

    #[test]
    fn unreadable_or_unknown_places_give_an_empty_label() {
        // No Claude Code folder at all.
        assert_eq!(Labels::new(None).label("abc-1"), SessionLabel::default());
        // A folder without projects.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            Labels::new(Some(empty.path())).label("abc-1"),
            SessionLabel::default()
        );
        // A transcript that is a folder: found, but not a file, so not found.
        let (dir, labels) = labels();
        std::fs::create_dir_all(dir.path().join("projects/-Users-x-tokred/abc-1.jsonl")).unwrap();
        assert_eq!(labels.label("abc-1"), SessionLabel::default());
        // A file that cannot be read is an empty label, not an error.
        assert_eq!(
            read_label(&dir.path().join("missing.jsonl"), 10),
            SessionLabel::default()
        );
        assert_eq!(
            read_label(&dir.path().join("missing.jsonl"), TAIL_BYTES + 10),
            SessionLabel::default()
        );
        assert_eq!(read_part(&dir.path().join("projects"), 0, 10), None);
    }

    #[test]
    fn a_poisoned_cache_still_answers() {
        let (dir, labels) = labels();
        write(&dir, "-Users-x-tokred", "abc-1", &format!("{TITLE}\n"));
        let labels = std::sync::Arc::new(labels);
        let held = std::sync::Arc::clone(&labels);
        let _ = std::thread::spawn(move || {
            let _guard = held.cache.lock().unwrap();
            panic!("poison the lock");
        })
        .join();
        assert!(labels.cache.is_poisoned());
        assert!(labels.label("abc-1").title.is_some());
    }
}
