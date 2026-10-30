//! `model-guard` (`PreModelSwitch`): what a model switch costs in prompt cache.

use tokred_core::config::Mode;
use tokred_core::hook_event::decision;

use crate::error::HookError;
use crate::io::{HookInput, ask_switch, system_message};
use crate::ports::{HookDeps, HookResponse};

/// `reason` of a switch the user was told about or asked to confirm.
pub const REASON_SWITCH_COSTLY: &str = "switch_costly";
/// `reason` of a switch with a context under `[model_guard] min_context_tokens`.
pub const REASON_BELOW_LIMIT: &str = "below_limit";
/// `reason` of a switch while the current model's prompt cache is not warm.
pub const REASON_CACHE_COLD: &str = "cache_cold";
/// `reason` of a "switch" to the model already in use.
pub const REASON_SAME_MODEL: &str = "same_model";

/// `model-guard`: when a switch would throw away a warm prompt cache of a large context, tells
/// the user what it costs (`warn`) or asks for a confirmation (`on`). Every other switch is left
/// to Claude Code, with no answer: `permissionDecision: "allow"` would skip its own confirmation.
///
/// The figures come from Claude Code in the hook input, so tokred estimates nothing here.
pub fn model_guard(input: &HookInput, deps: &HookDeps<'_>) -> Result<HookResponse, HookError> {
    let mode = deps.config.modes.model_guard;
    let session = input.session_id.as_str();
    if mode == Mode::Off || session.is_empty() {
        return Ok(HookResponse::silent());
    }
    let left_alone = |why| Ok(HookResponse::noted(session, decision::ALLOW, why, 0, None));

    let (from, to) = (input.from_model.as_deref(), input.to_model.as_deref());
    if from.is_some() && from == to {
        return left_alone(REASON_SAME_MODEL);
    }
    let tokens = input.context_tokens.unwrap_or(0);
    if tokens < deps.config.model_guard.min_context_tokens {
        return left_alone(REASON_BELOW_LIMIT);
    }
    if input.prompt_cache_warm != Some(true) {
        return left_alone(REASON_CACHE_COLD);
    }

    let cost = input
        .estimated_cache_write_usd
        .filter(|usd| usd.is_finite() && *usd >= 0.0)
        .map(|usd| format!(" (about ${usd:.2})"))
        .unwrap_or_default();
    let switch = format!(
        "from {} to {} re-sends about {}k tokens of context and rebuilds the prompt cache{cost}",
        from.unwrap_or("the current model"),
        to.unwrap_or("the new model"),
        tokens / 1000,
    );
    Ok(if mode == Mode::Enforce {
        let question = format!("Switching {switch}. Continue?");
        HookResponse::noted(
            session,
            decision::ASK,
            REASON_SWITCH_COSTLY,
            0,
            Some(ask_switch(&question)),
        )
    } else {
        HookResponse::noted(
            session,
            decision::WARN,
            REASON_SWITCH_COSTLY,
            0,
            Some(system_message(&format!("tokred: switching {switch}."))),
        )
    })
}
