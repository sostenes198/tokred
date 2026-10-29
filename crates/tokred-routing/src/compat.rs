//! Parameter compatibility: the request is turned into a list of needs (each beta,
//! each model-dependent field) and a target is acceptable only when it meets all of them.
//! Anything the catalog does not know is a need no model meets, so the request keeps its model.

use serde_json::Value;

use crate::catalog::{BetaNeed, Catalog, DefaultThinking, ModelInfo};

/// One thing the request asks of the model that serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Need {
    /// The target must have a capability of the catalog.
    Capability {
        /// Capability name.
        name: String,
        /// What in the request needs it.
        because: String,
    },
    /// The target must accept this `output_config.effort`.
    Effort(String),
    /// The target must accept this `max_tokens`.
    MaxOutput(u64),
    /// The request has no `thinking` field: the target must default to the same behaviour.
    DefaultThinking(DefaultThinking),
    /// Something the catalog does not know: no target meets it.
    Unknown(String),
}

impl Need {
    fn capability(name: &str, because: impl Into<String>) -> Self {
        Self::Capability {
            name: name.to_owned(),
            because: because.into(),
        }
    }

    /// Whether `target` meets this need.
    pub(crate) fn met_by(&self, target: &ModelInfo) -> bool {
        match self {
            Self::Capability { name, .. } => target.has(name),
            Self::Effort(level) => target.accepts_effort(level),
            Self::MaxOutput(tokens) => *tokens <= target.max_output,
            Self::DefaultThinking(default) => target.default_thinking == *default,
            Self::Unknown(_) => false,
        }
    }

    /// Why `target` cannot serve the request.
    pub(crate) fn refusal(&self, target: &str) -> String {
        match self {
            Self::Capability { name, because } => {
                format!("{target} lacks {name} (needed by {because})")
            }
            Self::Effort(level) => format!("{target} does not accept effort {level}"),
            Self::MaxOutput(tokens) => format!("{target} does not accept max_tokens {tokens}"),
            Self::DefaultThinking(default) => format!(
                "no thinking field and {target} does not default to {} thinking",
                default.as_str()
            ),
            Self::Unknown(what) => format!("{what} is not in the model catalog"),
        }
    }
}

/// Everything `body` and its `anthropic-beta` values ask of the model, given that it was
/// written for `source`.
pub(crate) fn needs(
    body: &Value,
    betas: &[String],
    source: &ModelInfo,
    catalog: &Catalog,
) -> Vec<Need> {
    let mut needs = Vec::new();
    for beta in betas {
        match catalog.beta_need(beta) {
            BetaNeed::Neutral => {}
            BetaNeed::Capability(name) => {
                needs.push(Need::capability(name, format!("anthropic-beta {beta}")));
            }
            BetaNeed::Unknown => needs.push(Need::Unknown(format!("anthropic-beta {beta}"))),
        }
    }
    if source.has("context_1m") {
        // Never shrink the context window: a long run could outgrow the smaller one.
        needs.push(Need::capability(
            "context_1m",
            format!("the 1M-token context of {}", source.id),
        ));
    }
    let Some(object) = body.as_object() else {
        needs.push(Need::Unknown("a body that is not an object".to_owned()));
        return needs;
    };
    let effort = object
        .get("output_config")
        .and_then(|config| config.get("effort"))
        .and_then(Value::as_str);
    thinking_needs(object.get("thinking"), effort, source, &mut needs);
    for (key, value) in object {
        match key.as_str() {
            "thinking" => {}
            "output_config" => output_config_needs(value, &mut needs),
            "context_management" => context_management_needs(value, &mut needs),
            "temperature" | "top_p" | "top_k" => needs.push(Need::capability("sampling", key)),
            "tool_choice" => {
                let kind = value.get("type").and_then(Value::as_str);
                if matches!(kind, Some("any" | "tool")) {
                    needs.push(Need::capability(
                        "forced_tool_choice",
                        "a forced tool_choice",
                    ));
                }
            }
            "tools" => tool_needs(value, &mut needs),
            "messages" => message_needs(value, &mut needs),
            "max_tokens" => needs.extend(value.as_u64().map(Need::MaxOutput)),
            _ if catalog.is_neutral_field(key) => {}
            other => needs.push(Need::Unknown(format!("the field {other}"))),
        }
    }
    needs
}

/// The first need `target` does not meet, if any.
pub(crate) fn first_unmet<'a>(needs: &'a [Need], target: &ModelInfo) -> Option<&'a Need> {
    needs.iter().find(|need| !need.met_by(target))
}

fn thinking_needs(
    thinking: Option<&Value>,
    effort: Option<&str>,
    source: &ModelInfo,
    needs: &mut Vec<Need>,
) {
    let Some(thinking) = thinking.filter(|t| !t.is_null()) else {
        needs.push(Need::DefaultThinking(source.default_thinking));
        return;
    };
    let Some(object) = thinking.as_object() else {
        needs.push(Need::Unknown(
            "a thinking value that is not an object".to_owned(),
        ));
        return;
    };
    // Turning thinking off is accepted by some models only up to `high` effort.
    let above_high = matches!(effort, Some("xhigh" | "max"));
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let capability = match kind {
        "adaptive" => "adaptive_thinking",
        "enabled" => "budget_thinking",
        "disabled" if above_high => "thinking_disabled",
        "disabled" => "thinking_disabled_at_high",
        "between_tools" if above_high => "between_tools",
        "between_tools" => "between_tools_at_high",
        other => {
            needs.push(Need::Unknown(format!("thinking.type {other:?}")));
            return;
        }
    };
    needs.push(Need::capability(
        capability,
        format!("thinking.type {kind}"),
    ));
    for (key, value) in object {
        match key.as_str() {
            "type" | "budget_tokens" => {}
            "display" if value.as_str() == Some("updates") => needs.push(Need::capability(
                "thinking_display_updates",
                "thinking.display updates",
            )),
            "display" => needs.push(Need::capability("thinking_display", "thinking.display")),
            other => needs.push(Need::Unknown(format!("thinking.{other}"))),
        }
    }
}

fn output_config_needs(config: &Value, needs: &mut Vec<Need>) {
    let Some(object) = config.as_object() else {
        needs.push(Need::Unknown(
            "an output_config that is not an object".to_owned(),
        ));
        return;
    };
    for (key, value) in object {
        match (key.as_str(), value.as_str()) {
            ("effort", Some(level)) => needs.push(Need::Effort(level.to_owned())),
            ("format", _) => needs.push(Need::capability(
                "structured_outputs",
                "output_config.format",
            )),
            ("task_budget", _) => {
                needs.push(Need::capability("task_budget", "output_config.task_budget"));
            }
            (other, _) => needs.push(Need::Unknown(format!("output_config.{other}"))),
        }
    }
}

fn context_management_needs(value: &Value, needs: &mut Vec<Need>) {
    needs.push(Need::capability("context_management", "context_management"));
    let Some(object) = value.as_object() else {
        needs.push(Need::Unknown(
            "a context_management that is not an object".to_owned(),
        ));
        return;
    };
    for (key, value) in object {
        let Some(edits) = value.as_array().filter(|_| key == "edits") else {
            needs.push(Need::Unknown(format!("context_management.{key}")));
            continue;
        };
        for edit in edits {
            let kind = edit.get("type").and_then(Value::as_str).unwrap_or_default();
            if kind.starts_with("clear_tool_uses") || kind.starts_with("clear_thinking") {
                // Covered by the `context_management` need above.
            } else if kind.starts_with("compact") {
                needs.push(Need::capability("compaction", format!("the edit {kind}")));
            } else {
                needs.push(Need::Unknown(format!("the context edit {kind:?}")));
            }
        }
    }
}

fn tool_needs(tools: &Value, needs: &mut Vec<Need>) {
    for tool in tools.as_array().into_iter().flatten() {
        // Server and Anthropic-defined tools carry a versioned `type` that depends on the model.
        match tool.get("type").and_then(Value::as_str) {
            None | Some("custom") => {}
            Some(kind) => needs.push(Need::Unknown(format!("the tool type {kind}"))),
        }
    }
}

fn message_needs(messages: &Value, needs: &mut Vec<Need>) {
    for message in messages.as_array().into_iter().flatten() {
        if message.get("role").and_then(Value::as_str) == Some("system") {
            needs.push(Need::capability(
                "system_messages",
                "a mid-conversation system message",
            ));
        }
        if message.get("output_config").is_some() {
            needs.push(Need::capability(
                "per_message_effort",
                "a per-message output_config",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn check(body: &Value, betas: &[&str], source: &str, target: &str) -> Option<String> {
        let catalog = Catalog::embedded();
        let source = catalog.model(source).unwrap();
        let target = catalog.model(target).unwrap();
        let betas: Vec<String> = betas.iter().map(|b| (*b).to_owned()).collect();
        let needs = needs(body, &betas, source, &catalog);
        first_unmet(&needs, target).map(|need| need.refusal(&target.id))
    }

    const SONNET: &str = "claude-sonnet-5-5";
    const HAIKU: &str = "claude-haiku-5-5";
    const OLD_HAIKU: &str = "claude-haiku-4-5";

    /// (body, betas, source, target, `None` = compatible or a fragment of the refusal).
    #[allow(clippy::type_complexity, clippy::too_many_lines)]
    fn cases() -> Vec<(
        Value,
        Vec<&'static str>,
        &'static str,
        &'static str,
        Option<&'static str>,
    )> {
        let base = |extra: Value| {
            let mut body = json!({
                "model": SONNET, "max_tokens": 32000, "stream": true,
                "system": [{"type": "text", "text": "s"}],
                "messages": [{"role": "user", "content": "find it"}],
                "tools": [{"name": "Read", "input_schema": {}}],
                "metadata": {"user_id": "u"},
                "thinking": {"type": "adaptive"}
            });
            for (k, v) in extra.as_object().unwrap() {
                body[k] = v.clone();
            }
            body
        };
        vec![
            // The plain Claude Code subagent request moves from Sonnet 5.5 to Haiku 5.5.
            (base(json!({})), vec![], SONNET, HAIKU, None),
            (
                base(json!({})),
                vec![
                    "oauth-2025-04-20",
                    "claude-code-20250219",
                    "context-1m-2025-08-07",
                ],
                SONNET,
                HAIKU,
                None,
            ),
            // Unknown beta, unknown field, unknown thinking type or key.
            (
                base(json!({})),
                vec!["brand-new-2027-01-01"],
                SONNET,
                HAIKU,
                Some("anthropic-beta brand-new"),
            ),
            (
                base(json!({"speed": "fast"})),
                vec![],
                SONNET,
                HAIKU,
                Some("the field speed"),
            ),
            (
                base(json!({"thinking": {"type": "sideways"}})),
                vec![],
                SONNET,
                HAIKU,
                Some("thinking.type \"sideways\""),
            ),
            (
                base(json!({"thinking": {"type": "adaptive", "block_binding": 1}})),
                vec![],
                SONNET,
                HAIKU,
                Some("thinking.block_binding"),
            ),
            (
                base(json!({"thinking": "on"})),
                vec![],
                SONNET,
                HAIKU,
                Some("thinking value"),
            ),
            // Adaptive thinking and a 1M context are not available on Haiku 4.5.
            (
                base(json!({})),
                vec![],
                SONNET,
                OLD_HAIKU,
                Some("context_1m"),
            ),
            (
                base(json!({})),
                vec![],
                "claude-opus-4-5",
                OLD_HAIKU,
                Some("adaptive_thinking"),
            ),
            (
                base(json!({"thinking": null})),
                vec![],
                SONNET,
                OLD_HAIKU,
                Some("context_1m"),
            ),
            // No thinking field: the default behaviour must match.
            (base(json!({"thinking": null})), vec![], SONNET, HAIKU, None),
            (
                base(json!({"thinking": null})),
                vec![],
                "claude-opus-4-8",
                HAIKU,
                Some("does not default to off"),
            ),
            (
                base(json!({"thinking": null})),
                vec![],
                "claude-opus-4-6",
                "claude-sonnet-4-6",
                None,
            ),
            // Manual budgets, disabled thinking and between_tools.
            (
                base(json!({"thinking": {"type": "enabled", "budget_tokens": 2000}})),
                vec![],
                "claude-sonnet-4-6",
                OLD_HAIKU,
                Some("context_1m"),
            ),
            (
                base(json!({"thinking": {"type": "enabled", "budget_tokens": 2000}})),
                vec![],
                "claude-sonnet-4-6",
                HAIKU,
                Some("budget_thinking"),
            ),
            (
                base(json!({"thinking": {"type": "disabled"}})),
                vec![],
                "claude-sonnet-5",
                HAIKU,
                None,
            ),
            (
                base(json!({"thinking": {"type": "disabled"}, "output_config": {"effort": "max"}})),
                vec![],
                "claude-sonnet-5",
                HAIKU,
                Some("thinking_disabled "),
            ),
            (
                base(json!({"thinking": {"type": "between_tools"}})),
                vec![],
                SONNET,
                HAIKU,
                Some("between_tools_at_high"),
            ),
            (
                base(
                    json!({"thinking": {"type": "between_tools"}, "output_config": {"effort": "xhigh"}}),
                ),
                vec![],
                SONNET,
                SONNET,
                Some("lacks between_tools "),
            ),
            // Display.
            (
                base(json!({"thinking": {"type": "adaptive", "display": "summarized"}})),
                vec![],
                SONNET,
                HAIKU,
                None,
            ),
            (
                base(json!({"thinking": {"type": "adaptive", "display": "updates"}})),
                vec![],
                SONNET,
                HAIKU,
                Some("thinking_display_updates"),
            ),
            // Effort and the rest of output_config.
            (
                base(json!({"output_config": {"effort": "xhigh"}})),
                vec![],
                SONNET,
                HAIKU,
                None,
            ),
            (
                base(json!({"output_config": {"effort": "xhigh"}})),
                vec![],
                "claude-opus-4-6",
                "claude-sonnet-4-6",
                Some("effort xhigh"),
            ),
            (
                base(json!({"output_config": {"format": {}, "task_budget": {}}})),
                vec![],
                SONNET,
                HAIKU,
                None,
            ),
            (
                base(json!({"output_config": {"task_budget": {}}})),
                vec![],
                SONNET,
                "claude-sonnet-5",
                Some("task_budget"),
            ),
            (
                base(json!({"output_config": {"effort": 3}})),
                vec![],
                SONNET,
                HAIKU,
                Some("output_config.effort"),
            ),
            (
                base(json!({"output_config": 1})),
                vec![],
                SONNET,
                HAIKU,
                Some("output_config that is not"),
            ),
            // Context management and compaction.
            (
                base(
                    json!({"context_management": {"edits": [{"type": "clear_thinking_20251015"}, {"type": "clear_tool_uses_20250919"}, {"type": "compact_20260112"}]}}),
                ),
                vec![],
                SONNET,
                HAIKU,
                None,
            ),
            (
                base(
                    json!({"thinking": {"type": "enabled", "budget_tokens": 2000}, "context_management": {"edits": [{"type": "compact_20260112"}]}}),
                ),
                vec![],
                "claude-opus-4-5",
                OLD_HAIKU,
                Some("compaction"),
            ),
            (
                base(json!({"context_management": {"edits": [{"type": "forget"}]}})),
                vec![],
                SONNET,
                HAIKU,
                Some("context edit \"forget\""),
            ),
            (
                base(json!({"context_management": {"other": []}})),
                vec![],
                SONNET,
                HAIKU,
                Some("context_management.other"),
            ),
            (
                base(json!({"context_management": {"edits": 1}})),
                vec![],
                SONNET,
                HAIKU,
                Some("context_management.edits"),
            ),
            (
                base(json!({"context_management": true})),
                vec![],
                SONNET,
                HAIKU,
                Some("context_management that is not"),
            ),
            // Sampling and forced tool use.
            (
                base(json!({"temperature": 0.2})),
                vec![],
                SONNET,
                HAIKU,
                Some("sampling"),
            ),
            (
                base(json!({"tool_choice": {"type": "auto"}})),
                vec![],
                SONNET,
                HAIKU,
                None,
            ),
            (
                base(json!({"tool_choice": {"type": "any"}})),
                vec![],
                SONNET,
                HAIKU,
                Some("forced_tool_choice"),
            ),
            // Typed tools, system messages and per-message effort.
            (
                base(
                    json!({"tools": [{"type": "custom", "name": "x"}, {"type": "web_search_20260209", "name": "web_search"}]}),
                ),
                vec![],
                SONNET,
                HAIKU,
                Some("web_search_20260209"),
            ),
            (
                base(
                    json!({"messages": [{"role": "user", "content": "x"}, {"role": "system", "content": "y"}]}),
                ),
                vec![],
                SONNET,
                HAIKU,
                None,
            ),
            (
                base(
                    json!({"messages": [{"role": "user", "content": "x"}, {"role": "system", "content": "y"}]}),
                ),
                vec![],
                SONNET,
                "claude-sonnet-5",
                Some("system_messages"),
            ),
            (
                base(
                    json!({"messages": [{"role": "system", "content": [], "output_config": {"effort": "low"}}]}),
                ),
                vec![],
                SONNET,
                "claude-sonnet-5",
                Some("system_messages"),
            ),
            (
                base(json!({"messages": [{"role": "user", "content": [], "output_config": {}}]})),
                vec![],
                SONNET,
                "claude-sonnet-5",
                Some("per_message_effort"),
            ),
            // max_tokens above the target's limit.
            (
                base(
                    json!({"thinking": {"type": "enabled", "budget_tokens": 2000}, "max_tokens": 100_000}),
                ),
                vec![],
                "claude-opus-4-5",
                OLD_HAIKU,
                Some("max_tokens 100000"),
            ),
            (
                base(json!({"max_tokens": "lots"})),
                vec![],
                SONNET,
                HAIKU,
                None,
            ),
        ]
    }

    #[test]
    fn the_compatibility_table_holds() {
        for (index, (body, betas, source, target, expected)) in cases().into_iter().enumerate() {
            let got = check(&body, &betas, source, target);
            assert_eq!(
                got.is_some(),
                expected.is_some(),
                "case {index}: got {got:?}"
            );
            let reason = got.unwrap_or_default();
            let fragment = expected.unwrap_or_default();
            assert!(
                reason.contains(fragment),
                "case {index}: {reason:?} lacks {fragment:?}"
            );
        }
    }

    #[test]
    fn a_body_that_is_not_an_object_is_never_routed() {
        assert!(
            check(&json!([1]), &[], SONNET, HAIKU)
                .unwrap()
                .contains("not an object")
        );
    }

    #[test]
    fn every_need_explains_itself() {
        let catalog = Catalog::embedded();
        let haiku = catalog.model(HAIKU).unwrap();
        let effort = Need::Effort("xhigh".to_owned());
        assert!(effort.met_by(haiku));
        assert_eq!(effort.refusal("m"), "m does not accept effort xhigh");
        assert!(Need::MaxOutput(10).met_by(haiku));
        assert!(Need::DefaultThinking(DefaultThinking::Adaptive).met_by(haiku));
        assert!(!Need::Unknown("x".to_owned()).met_by(haiku));
    }
}
