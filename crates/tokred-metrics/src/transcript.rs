//! Pure parsing of one transcript line into a usage record.
//!
//! Only `type: "assistant"` lines that carry `message.usage` and a real model produce a record;
//! everything else (user lines, attachments, queue operations, `<synthetic>` messages, malformed
//! JSON) yields `None`. Parsing is tolerant by design and never fails.

use serde::Deserialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokred_core::Timestamp;
use tokred_core::pricing::Usage;
use tokred_core::usage::UsageRecord;

/// What the file a line came from says about the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineContext<'a> {
    /// Project directory name.
    pub project: &'a str,
    /// Session id to use when the line has no `sessionId`.
    pub fallback_session: &'a str,
    /// The file lives under a `subagents/` directory.
    pub is_subagent_file: bool,
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    #[serde(rename = "isSidechain")]
    is_sidechain: Option<bool>,
    timestamp: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<RawUsage>,
}

#[derive(Deserialize)]
struct RawUsage {
    #[serde(rename = "input_tokens")]
    input: Option<u64>,
    #[serde(rename = "output_tokens")]
    output: Option<u64>,
    #[serde(rename = "cache_creation_input_tokens")]
    cache_write: Option<u64>,
    #[serde(rename = "cache_read_input_tokens")]
    cache_read: Option<u64>,
    /// Split of the cache writes by lifetime (`ephemeral_5m_input_tokens`,
    /// `ephemeral_1h_input_tokens`), present on current transcripts.
    cache_creation: Option<CacheCreation>,
}

#[derive(Deserialize)]
struct CacheCreation {
    #[serde(rename = "ephemeral_1h_input_tokens")]
    one_hour: Option<u64>,
}

/// Model id Claude Code gives to messages it fabricates locally (no tokens were spent).
const SYNTHETIC_MODEL: &str = "<synthetic>";

/// Extracts the usage of one JSONL line, or `None` if the line is not a billable assistant
/// message. The dedupe key is `"<message.id>:<requestId>"` (empty request id when absent).
#[must_use]
pub fn parse_line(line: &str, context: &LineContext<'_>) -> Option<UsageRecord> {
    let parsed: Line = serde_json::from_str(line).ok()?;
    if parsed.kind.as_deref() != Some("assistant") {
        return None;
    }
    let message = parsed.message?;
    let model = message.model.filter(|model| model != SYNTHETIC_MODEL)?;
    let raw = message.usage?;
    let id = message.id?;
    let ts = OffsetDateTime::parse(parsed.timestamp.as_deref()?, &Rfc3339).ok()?;
    Some(UsageRecord {
        message_key: format!("{id}:{}", parsed.request_id.unwrap_or_default()),
        session_id: parsed
            .session_id
            .unwrap_or_else(|| context.fallback_session.to_owned()),
        project: context.project.to_owned(),
        ts: Timestamp(ts.unix_timestamp()),
        model,
        usage: Usage {
            input: raw.input.unwrap_or(0),
            output: raw.output.unwrap_or(0),
            cache_write: raw.cache_write.unwrap_or(0),
            cache_write_1h: raw
                .cache_creation
                .and_then(|split| split.one_hour)
                .unwrap_or(0),
            cache_read: raw.cache_read.unwrap_or(0),
        },
        is_subagent: context.is_subagent_file || parsed.is_sidechain.unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTX: LineContext<'static> = LineContext {
        project: "-Users-me-app",
        fallback_session: "file-session",
        is_subagent_file: false,
    };

    fn assistant(extra: &str) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"s1","timestamp":"2026-03-01T10:00:00.789Z","requestId":"req_1","cwd":"/x","isSidechain":false,"message":{{"id":"msg_1","model":"claude-opus-4-1-20250805","usage":{{"input_tokens":10,"output_tokens":20,"cache_creation_input_tokens":30,"cache_read_input_tokens":40,"service_tier":"standard"}}}}{extra}}}"#
        )
    }

    #[test]
    fn assistant_lines_become_records() {
        let record = parse_line(&assistant(""), &CTX).unwrap();
        assert_eq!(record.message_key, "msg_1:req_1");
        assert_eq!(record.session_id, "s1");
        assert_eq!(record.project, "-Users-me-app");
        assert_eq!(record.ts, Timestamp(1_772_359_200));
        assert_eq!(record.model, "claude-opus-4-1-20250805");
        assert_eq!(
            record.usage,
            Usage {
                input: 10,
                output: 20,
                cache_write: 30,
                cache_write_1h: 0,
                cache_read: 40
            }
        );
        assert!(!record.is_subagent);
    }

    #[test]
    fn the_one_hour_part_of_cache_writes_is_read() {
        let line = assistant("").replace(
            r#""service_tier":"standard""#,
            r#""cache_creation":{"ephemeral_5m_input_tokens":10,"ephemeral_1h_input_tokens":20},"service_tier":"standard""#,
        );
        let record = parse_line(&line, &CTX).unwrap();
        assert_eq!(record.usage.cache_write, 30);
        assert_eq!(record.usage.cache_write_1h, 20);
        // An empty split object counts as no 1-hour writes.
        let empty = assistant("").replace(
            r#""service_tier":"standard""#,
            r#""cache_creation":{},"service_tier":"standard""#,
        );
        assert_eq!(parse_line(&empty, &CTX).unwrap().usage.cache_write_1h, 0);
    }

    #[test]
    fn sidechain_lines_and_subagent_files_are_flagged() {
        let sidechain = assistant("").replace("\"isSidechain\":false", "\"isSidechain\":true");
        assert!(parse_line(&sidechain, &CTX).unwrap().is_subagent);
        let in_subagent_file = LineContext {
            is_subagent_file: true,
            ..CTX
        };
        assert!(
            parse_line(&assistant(""), &in_subagent_file)
                .unwrap()
                .is_subagent
        );
    }

    #[test]
    fn missing_optional_fields_fall_back() {
        let line = r#"{"type":"assistant","timestamp":"2026-03-01T10:00:00Z","message":{"id":"m","model":"claude-haiku-4","usage":{"input_tokens":5}}}"#;
        let record = parse_line(line, &CTX).unwrap();
        assert_eq!(record.message_key, "m:");
        assert_eq!(record.session_id, "file-session");
        assert_eq!(
            record.usage,
            Usage {
                input: 5,
                ..Usage::default()
            }
        );
        assert!(!record.is_subagent);
    }

    #[test]
    fn everything_else_is_ignored() {
        let base = assistant("");
        let cases = [
            String::new(),
            "not json".to_owned(),
            "{\"type\":\"assistant\"".to_owned(),
            r#"{"type":"user","message":{"role":"user","content":"hi"}}"#.to_owned(),
            r#"{"type":"attachment","timestamp":"2026-03-01T10:00:00Z"}"#.to_owned(),
            r#"{"type":"queue-operation","operation":"enqueue"}"#.to_owned(),
            r#"{"timestamp":"2026-03-01T10:00:00Z"}"#.to_owned(),
            base.replace("claude-opus-4-1-20250805", SYNTHETIC_MODEL),
            base.replace("\"model\":\"claude-opus-4-1-20250805\",", ""),
            base.replace("\"usage\"", "\"usage_gone\""),
            base.replace("\"id\":\"msg_1\",", ""),
            base.replace("2026-03-01T10:00:00.789Z", "yesterday"),
            base.replace("\"timestamp\":\"2026-03-01T10:00:00.789Z\",", ""),
            base.replace("\"input_tokens\":10", "\"input_tokens\":-1"),
            r#"{"type":"assistant","message":"oops"}"#.to_owned(),
        ];
        for case in cases {
            assert_eq!(parse_line(&case, &CTX), None, "{case}");
        }
    }
}
