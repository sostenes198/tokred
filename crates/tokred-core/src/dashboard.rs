//! Domain of the live dashboard (`/dashboard` and `/metrics` of the proxy): the JSON shapes of
//! the dashboard API, the rules that decide whether a session is active, idle or ended, the
//! rows the store keeps for it, and the [`DashboardSource`] port the proxy reads history
//! through. Pure: no I/O, no async.
//!
//! The shapes are the contract shared with the browser app (`docs/dashboard.md`): change a name
//! here and the frontend breaks.

use serde::{Deserialize, Serialize};

use crate::pricing::Usage;
use crate::types::Timestamp;

/// A session with no request for this long is idle (it was active before).
pub const ACTIVE_WINDOW_SECS: i64 = 300;
/// A session with no request for this long has ended.
pub const ENDED_AFTER_SECS: i64 = 3600;
/// How many ended or idle sessions the dashboard lists next to the active ones.
pub const RECENT_SESSIONS: usize = 10;
/// The most turns one session detail carries (the newest ones; older turns stay in the store).
pub const MAX_TURNS: usize = 500;
/// Longest text the dashboard carries (messages, responses, tool summaries), in characters.
pub const MAX_TEXT_CHARS: usize = 20_000;
/// A subagent counts as running until this long after its last request (seconds).
pub const SUBAGENT_LINGER_SECS: i64 = 60;

/// Whether a session is being used right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// A request is in flight, or the last one was seen within [`ACTIVE_WINDOW_SECS`].
    Active,
    /// No request for [`ACTIVE_WINDOW_SECS`], but less than [`ENDED_AFTER_SECS`].
    Idle,
    /// No request for [`ENDED_AFTER_SECS`] or more.
    Ended,
}

/// The status of a session whose last activity was at `last_activity`.
#[must_use]
pub fn session_status(now: Timestamp, last_activity: Timestamp, in_flight: bool) -> SessionStatus {
    let quiet = now.0.saturating_sub(last_activity.0);
    if in_flight || quiet <= ACTIVE_WINDOW_SECS {
        SessionStatus::Active
    } else if quiet < ENDED_AFTER_SECS {
        SessionStatus::Idle
    } else {
        SessionStatus::Ended
    }
}

/// The origin name the dashboard shows: `main`, `subagent` or `auxiliary` (a compaction request
/// is shown as auxiliary).
#[must_use]
pub fn origin_label(origin: &str) -> &'static str {
    match origin {
        "main" => "main",
        "subagent" => "subagent",
        _ => "auxiliary",
    }
}

/// Text collected up to a character limit: what does not fit is counted, and
/// [`Clipped::finish`] ends the text with `…[truncated N chars]`. Cuts happen on character
/// boundaries.
#[derive(Debug, Clone)]
pub struct Clipped {
    limit: usize,
    text: String,
    kept: usize,
    dropped: usize,
}

impl Clipped {
    /// An empty text that keeps at most `limit` characters.
    #[must_use]
    pub const fn new(limit: usize) -> Self {
        Self {
            limit,
            text: String::new(),
            kept: 0,
            dropped: 0,
        }
    }

    /// Appends `piece`, keeping what fits and counting the rest.
    pub fn push_str(&mut self, piece: &str) {
        let room = self.limit - self.kept;
        let cut = piece
            .char_indices()
            .nth(room)
            .map_or(piece.len(), |(at, _)| at);
        let (head, tail) = piece.split_at(cut);
        self.kept += head.chars().count();
        self.dropped += tail.chars().count();
        self.text.push_str(head);
    }

    /// Nothing was pushed (or only empty text).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kept == 0 && self.dropped == 0
    }

    /// The text with the truncation marker when anything was dropped.
    #[must_use]
    pub fn finish(self) -> String {
        if self.dropped == 0 {
            self.text
        } else {
            format!("{}…[truncated {} chars]", self.text, self.dropped)
        }
    }
}

/// Cuts `text` to [`MAX_TEXT_CHARS`] characters (never in the middle of a character), ending
/// with `…[truncated N chars]` when something was cut.
#[must_use]
pub fn truncate_text(text: &str) -> String {
    let mut clipped = Clipped::new(MAX_TEXT_CHARS);
    clipped.push_str(text);
    clipped.finish()
}

/// One session (`S` of the API).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionView {
    /// Claude Code's session id.
    pub id: String,
    /// First request (unix seconds).
    pub started_at: i64,
    /// Latest request or response (unix seconds).
    pub last_activity_at: i64,
    /// `active`, `idle` or `ended`.
    pub status: SessionStatus,
    /// Requests made.
    pub requests: u64,
    /// Models that answered, in the order they were first seen.
    pub models: Vec<String>,
    /// Estimated cost in USD at the current price table.
    pub cost_usd: f64,
    /// Uncached input tokens.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Tokens read from the prompt cache.
    pub cache_read: u64,
    /// Tokens written to the prompt cache.
    pub cache_write: u64,
    /// Subagents running now.
    pub subagents_running: u64,
    /// Message contents are stored for this session (`[proxy] log_bodies`).
    pub content_recorded: bool,
    /// The name Claude Code shows for the session (the title you gave it, else the one it
    /// generated), when its transcript is known.
    #[serde(default)]
    pub title: Option<String>,
    /// The folder the session runs in (its last component), when its transcript is known.
    #[serde(default)]
    pub project: Option<String>,
}

/// The longest title kept, in characters.
pub const MAX_TITLE_CHARS: usize = 120;

/// What a session's transcript says about its name and where it runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionLabel {
    /// See [`SessionView::title`].
    pub title: Option<String>,
    /// See [`SessionView::project`].
    pub project: Option<String>,
}

/// Reads the name of a session out of (a part of) its transcript, which is JSON Lines.
///
/// The title is the last `custom-title` (you renamed the session), else the last `ai-title`
/// (the one Claude Code generated), else the last `agent-name`. The project is the last component of
/// the last `cwd`. A line that is cut off or is not JSON is skipped, so a slice that starts in
/// the middle of a line is fine.
#[must_use]
pub fn session_label(transcript: &str) -> SessionLabel {
    let (mut custom, mut generated, mut agent, mut cwd) = (None, None, None, None);
    for line in transcript.lines() {
        if let Some(path) = cwd_of(line) {
            cwd = Some(path);
        }
        let Some((slot, key)) = [
            (&mut custom, "customTitle"),
            (&mut generated, "aiTitle"),
            (&mut agent, "agentName"),
        ]
        .into_iter()
        .find(|(_, key)| line.contains(&format!("\"{key}\""))) else {
            continue;
        };
        let value: Option<serde_json::Value> = serde_json::from_str(line).ok();
        let text = value
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty());
        if let Some(text) = text {
            *slot = Some(text.chars().take(MAX_TITLE_CHARS).collect());
        }
    }
    SessionLabel {
        title: custom.or(generated).or(agent),
        project: cwd.and_then(|path: String| {
            path.rsplit(['/', '\\'])
                .find(|part| !part.is_empty())
                .map(str::to_owned)
        }),
    }
}

/// The value of the `"cwd"` key of a transcript line, when it has one (found without parsing the
/// whole line, which can be very large).
fn cwd_of(line: &str) -> Option<String> {
    let start = line.find("\"cwd\":\"")? + "\"cwd\":".len();
    let rest = &line[start..];
    let mut end = 1;
    let bytes = rest.as_bytes();
    while end < bytes.len() && bytes[end] != b'"' {
        end += if bytes[end] == b'\\' { 2 } else { 1 };
    }
    serde_json::from_str(rest.get(..=end)?).ok()
}

/// Token counts of a turn (`usage` of `T`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageView {
    /// Uncached input tokens.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Tokens written to the prompt cache.
    pub cache_write: u64,
    /// Tokens read from the prompt cache.
    pub cache_read: u64,
}

impl From<&Usage> for UsageView {
    fn from(usage: &Usage) -> Self {
        Self {
            input: usage.input,
            output: usage.output,
            cache_write: usage.cache_write,
            cache_read: usage.cache_read,
        }
    }
}

/// What the routing hook decided for a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutingView {
    /// `applied`, `observed`, `kept` or `fallback`.
    pub outcome: String,
    /// Why.
    pub reason: String,
}

/// A message Claude Code sent that was not part of the previous request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentMessage {
    /// `user`, `assistant` or `system`.
    pub role: String,
    /// The text, truncated to [`MAX_TEXT_CHARS`].
    pub text: String,
}

/// A tool the model called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolUseView {
    /// Tool name.
    pub name: String,
    /// A short summary of the input (never the whole input).
    pub input_summary: String,
}

/// What the model answered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseView {
    /// The reassembled text, truncated to [`MAX_TEXT_CHARS`].
    pub text: String,
    /// The reasoning text, when the response carried any.
    pub thinking: Option<String>,
    /// The tools called, in order.
    pub tool_uses: Vec<ToolUseView>,
}

/// The series of a request: the first characters of the hash of its system prompt (`None` when the
/// request had none recorded).
#[must_use]
pub fn series_of(system_hash: &str) -> Option<String> {
    (!system_hash.is_empty()).then(|| system_hash.chars().take(8).collect())
}

/// One request and its response (`T` of the API).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnView {
    /// The request id (also the primary key in the store).
    pub id: String,
    /// The session.
    pub session_id: String,
    /// When the request arrived (unix seconds).
    pub ts: i64,
    /// `main`, `subagent` or `auxiliary`.
    pub origin: String,
    /// The subagent that sent it.
    pub agent_id: Option<String>,
    /// The kind of subagent.
    pub agent_type: Option<String>,
    /// The model Claude Code asked for.
    pub model_requested: String,
    /// The model that answered (the model sent, while the turn is in flight).
    pub model_effective: String,
    /// HTTP status, once there is one.
    pub status: Option<u16>,
    /// Why the model stopped.
    pub stop_reason: Option<String>,
    /// Token counts.
    pub usage: UsageView,
    /// Estimated cost, once the turn is over and the model is priced.
    pub cost_usd: Option<f64>,
    /// Milliseconds from the request to the end of the response.
    pub duration_ms: Option<u64>,
    /// Milliseconds to the first upstream byte.
    pub ttfb_ms: Option<u64>,
    /// Names of the tools the model called.
    pub tools: Vec<String>,
    /// What the routing hook decided.
    pub routing: Option<RoutingView>,
    /// Why the prompt cache missed (`compaction`, `idle_ttl`, ...), for a main-loop request whose
    /// cache read collapsed.
    #[serde(default)]
    pub miss_cause: Option<String>,
    /// The first characters of the hash of the system prompt: requests of the same model with the
    /// same system prompt are one series, and each series has a cache of its own.
    #[serde(default)]
    pub series: Option<String>,
    /// The response has not ended.
    pub in_flight: bool,
    /// New messages of the request; `None` when contents are not recorded.
    pub sent: Option<Vec<SentMessage>>,
    /// The response; `None` while in flight or when contents are not recorded.
    pub response: Option<ResponseView>,
}

/// The sessions of `GET /dashboard/api/sessions`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionList {
    /// Sessions in use now, newest activity first.
    pub active: Vec<SessionView>,
    /// The last [`RECENT_SESSIONS`] idle or ended sessions, newest first.
    pub recent: Vec<SessionView>,
}

/// The body of `GET /dashboard/api/sessions/{id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDetail {
    /// The session.
    pub session: SessionView,
    /// Its turns, oldest first.
    pub turns: Vec<TurnView>,
    /// Older turns exist than the [`MAX_TURNS`] listed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // the signature serde's `skip_serializing_if` needs
fn is_false(value: &bool) -> bool {
    !*value
}

/// Data of the `turn_delta` event: text appended to the response of a turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TurnDelta {
    /// The session.
    pub session_id: String,
    /// The turn.
    pub turn_id: String,
    /// The text to append.
    pub text: String,
}

/// Whether a subagent is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SubagentState {
    /// A request of the agent is in flight or was seen recently.
    Running,
    /// The agent has been quiet for [`SUBAGENT_LINGER_SECS`].
    Finished,
}

/// Data of the `subagent` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SubagentEvent {
    /// The session.
    pub session_id: String,
    /// The subagent.
    pub agent_id: String,
    /// The kind of subagent, when known.
    pub agent_type: Option<String>,
    /// Running or finished.
    pub state: SubagentState,
    /// When the state changed (unix seconds).
    pub ts: i64,
}

/// What the proxy keeps of a turn besides the request record: facts that are not message
/// content (`stop_reason`, the tools called) and, only when `[proxy] log_bodies` is on, the
/// content. Stored in `captured_turns` (written by the proxy's single writer).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapturedTurn {
    /// The request id (`proxy_requests.id`).
    pub request_id: String,
    /// The session.
    pub session_id: String,
    /// The subagent, for subagent requests.
    pub agent_id: Option<String>,
    /// When the request arrived.
    pub ts: Timestamp,
    /// Why the model stopped.
    pub stop_reason: Option<String>,
    /// A JSON array with the names of the tools called.
    pub tools_json: String,
    /// A JSON array of [`SentMessage`]; `None` when contents are not recorded.
    pub sent_json: Option<String>,
    /// A JSON object of [`ResponseView`]; `None` when contents are not recorded.
    pub response_json: Option<String>,
}

/// The part of a `captured_turns` row the dashboard reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapturedRow {
    /// Why the model stopped.
    pub stop_reason: Option<String>,
    /// A JSON array with the names of the tools called.
    pub tools_json: String,
    /// A JSON array of [`SentMessage`].
    pub sent_json: Option<String>,
    /// A JSON object of [`ResponseView`].
    pub response_json: Option<String>,
}

/// One stored turn: the request record, what was captured and the routing decision.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredTurn {
    /// The `proxy_requests` row.
    pub record: crate::proxy::ProxyRequestRecord,
    /// The `captured_turns` row, when there is one.
    pub captured: Option<CapturedRow>,
    /// `(outcome, reason)` of the routing decision the request carries.
    pub routing: Option<(String, String)>,
}

/// What a session did with one model, summed from `proxy_requests`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionModelRow {
    /// The session.
    pub session_id: String,
    /// First request of the session.
    pub first_ts: Timestamp,
    /// Last request of the session.
    pub last_ts: Timestamp,
    /// The model that answered.
    pub model: String,
    /// Requests answered by this model.
    pub requests: u64,
    /// Tokens of those requests.
    pub usage: Usage,
    /// Turns of the whole session whose message contents are stored.
    pub content_turns: u64,
}

/// Where the proxy reads history and the report from. Every method may block (it reads the
/// database), so the proxy calls them off its async threads, except [`DashboardSource::cost_usd`],
/// which must be a lookup in memory.
pub trait DashboardSource: Send + Sync {
    /// The active sessions and the most recent idle or ended ones.
    fn sessions(&self) -> Result<SessionList, String>;

    /// A session and its newest [`MAX_TURNS`] turns, or `None` when the store does not know it.
    fn session(&self, id: &str) -> Result<Option<SessionDetail>, String>;

    /// The HTML report `tokred metrics` writes.
    fn metrics_html(&self) -> Result<String, String>;

    /// Estimated cost of `usage` for `model`; `None` when the model has no price.
    fn cost_usd(&self, model: &str, usage: &Usage) -> Option<f64>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(parts: &[&str]) -> String {
        parts.join("\n")
    }

    #[test]
    fn a_title_you_gave_beats_the_generated_one_and_the_last_one_wins() {
        let transcript = lines(&[
            r#"{"type":"ai-title","aiTitle":"First idea","sessionId":"s"}"#,
            r#"{"type":"agent-name","agentName":"Agent name","sessionId":"s"}"#,
            r#"{"type":"ai-title","aiTitle":"  Tokred spec implementation  ","sessionId":"s"}"#,
        ]);
        assert_eq!(
            session_label(&transcript).title.as_deref(),
            Some("Tokred spec implementation")
        );
        let renamed = lines(&[
            &transcript,
            r#"{"type":"custom-title","customTitle":"My name","sessionId":"s"}"#,
            r#"{"type":"ai-title","aiTitle":"Later generated","sessionId":"s"}"#,
        ]);
        assert_eq!(session_label(&renamed).title.as_deref(), Some("My name"));
        // Without either title the agent name is the name.
        let only_agent = r#"{"type":"agent-name","agentName":"Resolução de problema"}"#;
        assert_eq!(
            session_label(only_agent).title.as_deref(),
            Some("Resolução de problema")
        );
    }

    #[test]
    fn empty_cut_and_foreign_lines_are_skipped() {
        let transcript = lines(&[
            r#"nt":"half a line","aiTitle":"cut"#,
            "not json at all aiTitle",
            r#"{"type":"ai-title","aiTitle":"   ","sessionId":"s"}"#,
            r#"{"type":"ai-title","aiTitle":7,"sessionId":"s"}"#,
            r#"{"type":"user","note":"mentions \"aiTitle\" in text"}"#,
            "",
        ]);
        assert_eq!(session_label(&transcript), SessionLabel::default());
        assert_eq!(session_label(""), SessionLabel::default());
    }

    #[test]
    fn a_long_title_is_cut() {
        let long = "x".repeat(MAX_TITLE_CHARS + 50);
        let transcript = format!(r#"{{"type":"ai-title","aiTitle":"{long}"}}"#);
        let title = session_label(&transcript).title.unwrap();
        assert_eq!(title.chars().count(), MAX_TITLE_CHARS);
    }

    #[test]
    fn the_project_is_the_last_folder_of_the_last_cwd() {
        let transcript = lines(&[
            r#"{"type":"user","cwd":"/Users/x/Repos/old","sessionId":"s"}"#,
            r#"{"type":"assistant","message":{"text":"..."},"cwd":"/Users/x/Repos/tokred/","sessionId":"s"}"#,
        ]);
        assert_eq!(
            session_label(&transcript).project.as_deref(),
            Some("tokred")
        );
        let windows = r#"{"type":"user","cwd":"C:\\Users\\x\\my repo","sessionId":"s"}"#;
        assert_eq!(session_label(windows).project.as_deref(), Some("my repo"));
        let escaped = r#"{"type":"user","cwd":"/a/b \"quoted\"","sessionId":"s"}"#;
        assert_eq!(
            session_label(escaped).project.as_deref(),
            Some("b \"quoted\"")
        );
        // Not a string, unterminated, or the root: no project.
        for bad in [
            r#"{"cwd":5}"#,
            r#"{"cwd":"/never/ends"#,
            r#"{"cwd":"/"}"#,
            r#"{"cwd":""}"#,
        ] {
            assert_eq!(session_label(bad).project, None, "{bad}");
        }
    }

    #[test]
    fn the_status_follows_the_quiet_time() {
        let now = Timestamp(10_000);
        let at = |ago: i64| Timestamp(now.0 - ago);
        assert_eq!(session_status(now, at(0), false), SessionStatus::Active);
        assert_eq!(
            session_status(now, at(ACTIVE_WINDOW_SECS), false),
            SessionStatus::Active
        );
        assert_eq!(
            session_status(now, at(ACTIVE_WINDOW_SECS + 1), false),
            SessionStatus::Idle
        );
        assert_eq!(
            session_status(now, at(ENDED_AFTER_SECS - 1), false),
            SessionStatus::Idle
        );
        assert_eq!(
            session_status(now, at(ENDED_AFTER_SECS), false),
            SessionStatus::Ended
        );
        assert_eq!(
            session_status(now, at(ENDED_AFTER_SECS * 5), true),
            SessionStatus::Active,
            "a request in flight keeps a session active"
        );
        assert_eq!(
            session_status(now, Timestamp(now.0 + 50), false),
            SessionStatus::Active,
            "a clock step backwards is not a quiet session"
        );
        assert_eq!(
            serde_json::to_string(&SessionStatus::Idle).unwrap(),
            "\"idle\""
        );
    }

    #[test]
    fn origins_are_shown_as_main_subagent_or_auxiliary() {
        assert_eq!(origin_label("main"), "main");
        assert_eq!(origin_label("subagent"), "subagent");
        assert_eq!(origin_label("auxiliary"), "auxiliary");
        assert_eq!(origin_label("compaction"), "auxiliary");
    }

    #[test]
    fn long_text_is_cut_on_a_character_boundary_with_a_marker() {
        assert_eq!(truncate_text("short"), "short");
        let exact = "é".repeat(MAX_TEXT_CHARS);
        assert_eq!(truncate_text(&exact), exact, "the limit itself is kept");
        let long = format!("{exact}日本語");
        let cut = truncate_text(&long);
        assert!(cut.starts_with(&exact));
        assert!(cut.ends_with("…[truncated 3 chars]"), "{cut}");
    }

    #[test]
    fn clipped_text_counts_what_it_drops_across_pieces() {
        let mut clipped = Clipped::new(5);
        assert!(clipped.is_empty());
        clipped.push_str("ab");
        clipped.push_str("");
        clipped.push_str("cdéfg");
        clipped.push_str("hi");
        assert!(!clipped.is_empty());
        assert_eq!(clipped.finish(), "abcdé…[truncated 4 chars]");
        let mut fits = Clipped::new(3);
        fits.push_str("abc");
        assert_eq!(fits.finish(), "abc");
    }

    #[test]
    fn a_detail_says_so_only_when_it_is_truncated() {
        let session = SessionView {
            id: "s".into(),
            started_at: 1,
            last_activity_at: 2,
            status: SessionStatus::Ended,
            requests: 0,
            models: vec![],
            cost_usd: 0.0,
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            subagents_running: 0,
            content_recorded: false,
            title: None,
            project: None,
        };
        let mut detail = SessionDetail {
            session,
            turns: vec![],
            truncated: false,
        };
        assert!(
            serde_json::to_value(&detail)
                .unwrap()
                .get("truncated")
                .is_none(),
            "optional for the frontend"
        );
        detail.truncated = true;
        assert_eq!(serde_json::to_value(&detail).unwrap()["truncated"], true);
        let back: SessionDetail =
            serde_json::from_str(&serde_json::to_string(&detail).unwrap()).unwrap();
        assert_eq!(back, detail);
    }

    #[test]
    fn usage_views_leave_out_the_one_hour_split() {
        let usage = Usage {
            input: 1,
            output: 2,
            cache_write: 3,
            cache_write_1h: 2,
            cache_read: 4,
        };
        assert_eq!(
            UsageView::from(&usage),
            UsageView {
                input: 1,
                output: 2,
                cache_write: 3,
                cache_read: 4
            }
        );
    }

    #[test]
    fn events_serialize_to_the_documented_shapes() {
        let delta = TurnDelta {
            session_id: "s".into(),
            turn_id: "t".into(),
            text: "x".into(),
        };
        assert_eq!(
            serde_json::to_value(&delta).unwrap(),
            serde_json::json!({"session_id": "s", "turn_id": "t", "text": "x"})
        );
        let event = SubagentEvent {
            session_id: "s".into(),
            agent_id: "a".into(),
            agent_type: None,
            state: SubagentState::Finished,
            ts: 5,
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::json!({"session_id": "s", "agent_id": "a", "agent_type": null, "state": "finished", "ts": 5})
        );
        assert_eq!(
            serde_json::to_string(&SubagentState::Running).unwrap(),
            "\"running\""
        );
    }

    #[test]
    fn a_series_is_the_start_of_the_system_hash() {
        assert_eq!(series_of(""), None);
        assert_eq!(series_of("abc").as_deref(), Some("abc"));
        assert_eq!(series_of("0123456789abcdef").as_deref(), Some("01234567"));
    }
}
