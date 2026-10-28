//! Sessions, file reads and prompt classifications (tables `sessions`, `file_reads` and
//! `classifications`) and the ports the hooks use to persist them.

use crate::error::StoreError;
use crate::types::Timestamp;

/// The kind of task a prompt asks for (the `decision` of a `route` hook event).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskClass {
    /// Architecture and design decisions.
    Architecture,
    /// Planning and breaking down work.
    Planning,
    /// Implementation, refactoring and bug fixing.
    Implementation,
    /// Code and test review.
    Review,
    /// Locating code, files and symbols.
    Search,
    /// Summarizing documents, logs and long output.
    Summary,
    /// Anything else (including prompts too small to be worth delegating).
    Other,
}

impl TaskClass {
    /// Every class.
    pub const ALL: [Self; 7] = [
        Self::Architecture,
        Self::Planning,
        Self::Implementation,
        Self::Review,
        Self::Search,
        Self::Summary,
        Self::Other,
    ];

    /// Value stored in the database.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Architecture => "architecture",
            Self::Planning => "planning",
            Self::Implementation => "implementation",
            Self::Review => "review",
            Self::Search => "search",
            Self::Summary => "summary",
            Self::Other => "other",
        }
    }

    /// Parses the stored value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|class| class.as_str() == value)
    }

    /// The tokred subagent meant for this class; `None` for [`TaskClass::Other`].
    #[must_use]
    pub const fn agent(self) -> Option<&'static str> {
        match self {
            Self::Architecture => Some("tokred-architect"),
            Self::Planning => Some("tokred-planner"),
            Self::Implementation => Some("tokred-dev"),
            Self::Review => Some("tokred-reviewer"),
            Self::Search => Some("tokred-explorer"),
            Self::Summary => Some("tokred-summarizer"),
            Self::Other => None,
        }
    }

    /// Whether `agent` is an acceptable choice for this class.
    ///
    /// Architecture and planning both run on opus, so either agent fits either class.
    #[must_use]
    pub fn accepts(self, agent: &str) -> bool {
        match self {
            Self::Architecture | Self::Planning => {
                matches!(agent, "tokred-architect" | "tokred-planner")
            }
            other => other.agent() == Some(agent),
        }
    }
}

/// A Claude Code session (row of `sessions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Session id.
    pub id: String,
    /// Working directory the session started in.
    pub project: Option<String>,
    /// First time a hook saw the session.
    pub started_at: Timestamp,
    /// Last time a hook saw the session.
    pub last_seen: Timestamp,
    /// Last `clear`/`compact`: reads before it are no longer in the model's context.
    pub last_reset_at: Option<Timestamp>,
}

/// A file read recorded by `guard-read` (row of `file_reads`).
///
/// `offset` and `limit` are the line-based arguments of the `Read` tool; `None` means the
/// argument was absent (stored as `-1`, because SQLite treats `NULL`s in a key as distinct).
/// `session_id` is the *scope* of the read: the session id, followed by `/<agent_id>` for
/// reads made inside a subagent, whose context is separate from its parent's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRead {
    /// Session (and subagent) scope.
    pub session_id: String,
    /// Path as given to the `Read` tool.
    pub path: String,
    /// blake3 hash (hex) of the file content.
    pub hash: String,
    /// First line requested.
    pub offset: Option<u64>,
    /// Number of lines requested.
    pub limit: Option<u64>,
    /// Estimated bytes the read puts in context.
    pub bytes: u64,
    /// When the read happened.
    pub ts: Timestamp,
}

/// The class of a prompt (row of `classifications`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    /// Session id.
    pub session_id: String,
    /// When the prompt was classified.
    pub ts: Timestamp,
    /// The class.
    pub class: TaskClass,
    /// Prompt length in characters.
    pub prompt_len: u64,
}

/// Persistent sessions.
pub trait SessionRepo {
    /// Registers the session or refreshes `last_seen` (and `project`, when given).
    fn touch_session(
        &self,
        id: &str,
        project: Option<&str>,
        now: Timestamp,
    ) -> Result<(), StoreError>;
    /// Marks a `clear`/`compact` cut at `now`, registering the session if needed.
    fn mark_reset(&self, id: &str, now: Timestamp) -> Result<(), StoreError>;
    /// Looks the session up.
    fn get_session(&self, id: &str) -> Result<Option<Session>, StoreError>;
}

/// Persistent file reads.
pub trait FileReadRepo {
    /// The read of `path` with this exact content hash and range, if any.
    fn find_read(
        &self,
        scope: &str,
        path: &str,
        hash: &str,
        offset: Option<u64>,
        limit: Option<u64>,
    ) -> Result<Option<FileRead>, StoreError>;
    /// Inserts the read, or refreshes `bytes` and `ts` of an identical one.
    fn record_read(&self, read: &FileRead) -> Result<(), StoreError>;
}

/// Persistent classifications.
pub trait ClassificationRepo {
    /// The most recent classification of the session.
    fn last_classification(&self, session_id: &str) -> Result<Option<Classification>, StoreError>;
    /// Stores a classification.
    fn insert_classification(&self, classification: &Classification) -> Result<(), StoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_round_trip_and_map_to_agents() {
        for class in TaskClass::ALL {
            assert_eq!(TaskClass::parse(class.as_str()), Some(class));
            assert_eq!(
                class.agent().is_none(),
                class == TaskClass::Other,
                "{class:?}"
            );
        }
        assert_eq!(TaskClass::parse("nope"), None);
    }

    #[test]
    fn architecture_and_planning_accept_either_opus_agent() {
        assert!(TaskClass::Architecture.accepts("tokred-planner"));
        assert!(TaskClass::Planning.accepts("tokred-architect"));
        assert!(!TaskClass::Planning.accepts("tokred-dev"));
        assert!(TaskClass::Implementation.accepts("tokred-dev"));
        assert!(!TaskClass::Implementation.accepts("tokred-explorer"));
        assert!(!TaskClass::Other.accepts("tokred-dev"));
    }
}
