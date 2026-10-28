//! `session-start` (`SessionStart`) and `session-check` (Stop).

use std::path::Path;

use tokred_core::cache::{TierPolicy, cache_peek, cache_put};
use tokred_core::config::Mode;
use tokred_core::hook_event::decision;

use crate::error::HookError;
use crate::io::{HookInput, system_message};
use crate::ports::{HookDeps, HookResponse};

/// Kind of the "already warned" rows in the cache.
const SESSION_WARN_KIND: &str = "session_warn";

/// Sizes of the transcript tail that is searched for the last assistant message.
const TAIL_SIZES: [u64; 2] = [256 * 1024, 4 * 1024 * 1024];

/// `reason` of a `session-check` that warned the user.
pub const REASON_CONTEXT_LARGE: &str = "context_large";
/// `reason` of a `session-check` that stayed quiet because it already warned.
pub const REASON_ALREADY_WARNED: &str = "already_warned";
/// `reason` of a `session-check` below the limit.
pub const REASON_BELOW_LIMIT: &str = "below_limit";

/// `session-start` (spec 7.6): registers the session and, on `clear`/`compact`, marks a cut so
/// that `guard-read` allows reads again. It prints nothing: on `SessionStart` anything written
/// to stdout would be added to the model's context. It has no mode of its own.
pub fn session_start(input: &HookInput, deps: &HookDeps<'_>) -> Result<HookResponse, HookError> {
    let session = input.session_id.as_str();
    if session.is_empty() {
        return Ok(HookResponse::silent());
    }
    let now = deps.clock.now();
    deps.sessions
        .touch_session(session, input.cwd.as_deref(), now)?;
    let source = input.source.as_deref().unwrap_or_default();
    if matches!(source, "clear" | "compact") {
        deps.sessions.mark_reset(session, now)?;
    }
    // `fork` and `resume` keep the earlier context, so they are not cuts.
    let source = match source {
        "startup" => "startup",
        "resume" => "resume",
        "fork" => "fork",
        "clear" => "clear",
        "compact" => "compact",
        _ => "unknown",
    };
    Ok(HookResponse::noted(
        session,
        decision::NONE,
        source,
        0,
        None,
    ))
}

/// Tokens in the context of the last main-thread assistant message of a transcript tail:
/// `input + cache_read + cache_creation`. Subagent (`isSidechain`) lines and lines without
/// usage (or with an all-zero one, such as synthetic messages) are skipped.
#[must_use]
pub fn last_context_tokens(tail: &[u8]) -> Option<u64> {
    tail.split(|byte| *byte == b'\n').rev().find_map(|line| {
        // Cheap pre-filter before parsing a possibly huge JSON line.
        if !line.windows(7).any(|w| w == b"\"usage\"") {
            return None;
        }
        let value: serde_json::Value = serde_json::from_slice(line).ok()?;
        let sidechain = value
            .get("isSidechain")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if value.get("type")?.as_str()? != "assistant" || sidechain {
            return None;
        }
        let usage = value.get("message")?.get("usage")?;
        let field = |name: &str| usage.get(name).and_then(serde_json::Value::as_u64);
        let total = field("input_tokens").unwrap_or(0)
            + field("cache_read_input_tokens").unwrap_or(0)
            + field("cache_creation_input_tokens").unwrap_or(0);
        (total > 0).then_some(total)
    })
}

/// `session-check` (spec 7.5): above `context_warn_tokens`, shows the *user* (never the model)
/// a suggestion to /compact or /clear. It never blocks the stop and warns once per session,
/// again only after the context grew by half the limit or after a clear/compact.
///
/// The transcript is written asynchronously, so the numbers can lag the current turn by one
/// message; that is fine for a coarse warning.
pub fn session_check(input: &HookInput, deps: &HookDeps<'_>) -> Result<HookResponse, HookError> {
    let session = input.session_id.as_str();
    let Some(transcript) = input.transcript_path.as_deref() else {
        return Ok(HookResponse::silent());
    };
    if deps.config.modes.session_check == Mode::Off || session.is_empty() {
        return Ok(HookResponse::silent());
    }
    let mut tokens = None;
    for size in TAIL_SIZES {
        let Some(tail) = deps.files.read_tail(Path::new(transcript), size)? else {
            return Ok(HookResponse::silent());
        };
        tokens = last_context_tokens(&tail);
        if tokens.is_some() || (tail.len() as u64) < size {
            break;
        }
    }
    let Some(tokens) = tokens else {
        return Ok(HookResponse::silent());
    };

    let limit = deps.config.session_check.context_warn_tokens;
    if tokens <= limit {
        return Ok(HookResponse::noted(
            session,
            decision::ALLOW,
            REASON_BELOW_LIMIT,
            0,
            None,
        ));
    }
    let now = deps.clock.now();
    let cut = deps
        .sessions
        .get_session(session)?
        .and_then(|s| s.last_reset_at)
        .map_or(0, |t| t.0);
    let key = format!("{SESSION_WARN_KIND}:{session}:{cut}");
    let warned_at = cache_peek(deps.cache, &key, now)?
        .and_then(|entry| String::from_utf8(entry.value).ok()?.parse::<u64>().ok());
    if warned_at.is_some_and(|before| tokens < before.saturating_add(limit / 2)) {
        return Ok(HookResponse::noted(
            session,
            decision::ALLOW,
            REASON_ALREADY_WARNED,
            0,
            None,
        ));
    }
    let stored = cache_put(
        deps.cache,
        &key,
        SESSION_WARN_KIND,
        tokens.to_string().into_bytes(),
        now,
        &TierPolicy::from_config(&deps.config.cache),
    );
    stored?;
    let message = format!(
        "tokred: the context is about {}k tokens (limit {}k). When this task is wrapped up, \
         consider /compact to keep the thread or /clear to start fresh.",
        tokens / 1000,
        limit / 1000
    );
    Ok(HookResponse::noted(
        session,
        decision::WARN,
        REASON_CONTEXT_LARGE,
        0,
        Some(system_message(&message)),
    ))
}
