//! `route` (`UserPromptSubmit`) and `enforce` (`PreToolUse` on the subagent tool).

use tokred_core::cache::{TierPolicy, cache_get, cache_put, content_key};
use tokred_core::config::Mode;
use tokred_core::hook_event::{decision, reason};
use tokred_core::session::{Classification, TaskClass};

use crate::classify::{Rules, classify, suggestion};
use crate::error::HookError;
use crate::io::{HookInput, context, deny};
use crate::ports::{HookDeps, HookResponse};

/// `route` (spec 7.1): classifies the prompt and, when the class changed since the previous
/// actionable prompt of the session, suggests the right subagent in one line.
///
/// Prompts that classify as `other` (including short one-off prompts) are recorded as a
/// `route` event but do not become the session's "last classification": a "yes, go ahead"
/// between two implementation prompts must not make the suggestion repeat.
pub fn route(input: &HookInput, deps: &HookDeps<'_>) -> Result<HookResponse, HookError> {
    let mode = deps.config.modes.route;
    let session = input.session_id.as_str();
    let Some(prompt) = input.prompt.as_deref() else {
        return Ok(HookResponse::silent());
    };
    if mode == Mode::Off || session.is_empty() {
        return Ok(HookResponse::silent());
    }

    let class = cached_class(prompt, deps)?;
    let previous = deps.classes.last_classification(session)?.map(|c| c.class);
    let changed = class != TaskClass::Other && previous != Some(class);
    if class != TaskClass::Other {
        deps.classes.insert_classification(&Classification {
            session_id: session.to_owned(),
            ts: deps.clock.now(),
            class,
            prompt_len: prompt.chars().count() as u64,
        })?;
    }
    let output = if changed {
        suggestion(class).map(|line| context("UserPromptSubmit", &line))
    } else {
        None
    };
    let why = if changed {
        reason::SUGGESTED
    } else {
        reason::UNCHANGED
    };
    Ok(HookResponse::noted(session, class.as_str(), why, 0, output))
}

/// `enforce` (spec 7.2): compares the chosen tokred subagent with the session's last
/// classification. Agents that are not tokred's are always allowed.
pub fn enforce(input: &HookInput, deps: &HookDeps<'_>) -> Result<HookResponse, HookError> {
    let mode = deps.config.modes.enforce;
    let session = input.session_id.as_str();
    let is_agent_tool = matches!(input.tool_name.as_deref(), Some("Agent" | "Task"));
    // Only the main conversation is checked: its classification says nothing about a
    // subagent that delegates further.
    if mode == Mode::Off || !is_agent_tool || session.is_empty() || input.agent_id.is_some() {
        return Ok(HookResponse::silent());
    }
    let chosen = input.tool_str("subagent_type").unwrap_or_default();
    if !chosen.starts_with("tokred-") {
        return Ok(HookResponse::noted(session, decision::ALLOW, "", 0, None));
    }
    let class = deps
        .classes
        .last_classification(session)?
        .map_or(TaskClass::Other, |c| c.class);
    let Some(expected) = class.agent() else {
        return Ok(HookResponse::noted(session, decision::ALLOW, "", 0, None));
    };
    if class.accepts(chosen) {
        return Ok(HookResponse::noted(
            session,
            decision::ALLOW,
            reason::AGENT_MATCHES,
            0,
            None,
        ));
    }
    let class = class.as_str();
    Ok(if mode == Mode::Enforce {
        let message = format!(
            "tokred: this task was classified as {class}; delegate it to {expected} instead of {chosen}."
        );
        HookResponse::noted(
            session,
            decision::DENY,
            reason::AGENT_MISMATCH,
            0,
            Some(deny(&message)),
        )
    } else {
        let message = format!(
            "tokred: this task was classified as {class}; {expected} fits better than {chosen}."
        );
        HookResponse::noted(
            session,
            decision::WARN,
            reason::AGENT_MISMATCH,
            0,
            Some(context("PreToolUse", &message)),
        )
    })
}

/// Kind of the prompt-classification rows in the cache.
const CLASSIFICATION_KIND: &str = "classification";

/// Bump when the keyword rules change, so cached classes of older rules are not reused.
const RULES_VERSION: &str = "v1";

/// The class of a prompt, from the tiered cache when the same prompt was classified before
/// (the key holds the blake3 hash of the prompt), else classified and cached.
fn cached_class(prompt: &str, deps: &HookDeps<'_>) -> Result<TaskClass, HookError> {
    let now = deps.clock.now();
    let policy = TierPolicy::from_config(&deps.config.cache);
    let hash = blake3::hash(prompt.as_bytes()).to_hex().to_string();
    let key = content_key(CLASSIFICATION_KIND, RULES_VERSION, &hash);
    if let Some(class) = cache_get(deps.cache, &key, now, &policy)?
        .and_then(|entry| String::from_utf8(entry.value).ok())
        .and_then(|text| TaskClass::parse(&text))
    {
        return Ok(class);
    }
    let class = classify(prompt, &Rules::default());
    let stored = cache_put(
        deps.cache,
        &key,
        CLASSIFICATION_KIND,
        class.as_str().as_bytes().to_vec(),
        now,
        &policy,
    );
    stored?;
    Ok(class)
}
