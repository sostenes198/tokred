//! The "simple prompt" criteria, applied retrospectively to a finished prompt.
//!
//! Only tool *names* reach the proxy's records (never their arguments), so "a small edit" is
//! approximated as "at most one edit tool call".

/// What a finished prompt did, as far as the criteria are concerned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PromptFacts {
    /// Main-loop requests.
    pub turns: u32,
    /// Output tokens of the main loop and its subagents.
    pub output_tokens: u64,
    /// Thinking tokens of the main loop and its subagents.
    pub thinking_tokens: u64,
    /// Edit tool calls (`Edit`, `Write`, `MultiEdit`, `NotebookEdit`).
    pub edits: u32,
    /// Subagents started (`Agent` or `Task` tool calls).
    pub subagents: u32,
}

/// Thresholds of a "simple" prompt. A prompt is simple when it stays within **every** limit.
///
/// `[suggest]` in `tokred.toml` has no keys for these yet, so the defaults below apply:
///
/// | Limit | Default | Why |
/// |---|---|---|
/// | `max_turns` | 6 | a question, a few reads and an answer |
/// | `max_output_tokens` | 4000 | a short answer or a small patch |
/// | `max_thinking_tokens` | 2000 | little reasoning was needed |
/// | `max_edits` | 1 | read-only, or one small edit |
/// | `max_subagents` | 0 | delegating work is a sign of a larger task |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimpleCriteria {
    /// Most main-loop requests.
    pub max_turns: u32,
    /// Most output tokens.
    pub max_output_tokens: u64,
    /// Most thinking tokens.
    pub max_thinking_tokens: u64,
    /// Most edit tool calls.
    pub max_edits: u32,
    /// Most subagents started.
    pub max_subagents: u32,
}

impl Default for SimpleCriteria {
    fn default() -> Self {
        Self {
            max_turns: 6,
            max_output_tokens: 4000,
            max_thinking_tokens: 2000,
            max_edits: 1,
            max_subagents: 0,
        }
    }
}

impl SimpleCriteria {
    /// Names of the limits the prompt exceeded (`turns`, `output`, `thinking`, `edits`,
    /// `subagents`); empty when the prompt was simple.
    #[must_use]
    pub fn exceeded(&self, facts: &PromptFacts) -> Vec<&'static str> {
        [
            ("turns", facts.turns > self.max_turns),
            ("output", facts.output_tokens > self.max_output_tokens),
            ("thinking", facts.thinking_tokens > self.max_thinking_tokens),
            ("edits", facts.edits > self.max_edits),
            ("subagents", facts.subagents > self.max_subagents),
        ]
        .into_iter()
        .filter_map(|(name, over)| over.then_some(name))
        .collect()
    }

    /// The prompt stayed within every limit.
    #[must_use]
    pub fn is_simple(&self, facts: &PromptFacts) -> bool {
        self.exceeded(facts).is_empty()
    }
}

/// Tools that change files.
#[must_use]
pub fn is_edit_tool(name: &str) -> bool {
    matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit")
}

/// Tools that start a subagent.
#[must_use]
pub fn is_subagent_tool(name: &str) -> bool {
    matches!(name, "Agent" | "Task")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(turns: u32, output: u64, thinking: u64, edits: u32, subagents: u32) -> PromptFacts {
        PromptFacts {
            turns,
            output_tokens: output,
            thinking_tokens: thinking,
            edits,
            subagents,
        }
    }

    #[test]
    fn the_defaults_are_documented() {
        let c = SimpleCriteria::default();
        assert_eq!(
            (
                c.max_turns,
                c.max_output_tokens,
                c.max_thinking_tokens,
                c.max_edits,
                c.max_subagents
            ),
            (6, 4000, 2000, 1, 0)
        );
    }

    #[test]
    fn simple_and_not_simple_prompts() {
        let c = SimpleCriteria::default();
        let table: [(PromptFacts, &[&str]); 9] = [
            (facts(1, 300, 0, 0, 0), &[]),
            (facts(6, 4000, 2000, 1, 0), &[]),
            (PromptFacts::default(), &[]),
            (facts(7, 100, 0, 0, 0), &["turns"]),
            (facts(2, 4001, 0, 0, 0), &["output"]),
            (facts(2, 100, 2001, 0, 0), &["thinking"]),
            (facts(3, 900, 0, 2, 0), &["edits"]),
            (facts(3, 900, 0, 0, 1), &["subagents"]),
            (
                facts(40, 30_000, 9000, 12, 3),
                &["turns", "output", "thinking", "edits", "subagents"],
            ),
        ];
        for (prompt, expected) in table {
            assert_eq!(c.exceeded(&prompt), expected, "{prompt:?}");
            assert_eq!(c.is_simple(&prompt), expected.is_empty(), "{prompt:?}");
        }
        let loose = SimpleCriteria {
            max_subagents: 1,
            ..c
        };
        assert!(loose.is_simple(&facts(3, 900, 0, 0, 1)));
    }

    #[test]
    fn tool_kinds() {
        for edit in ["Edit", "Write", "MultiEdit", "NotebookEdit"] {
            assert!(is_edit_tool(edit));
        }
        assert!(!is_edit_tool("Read") && !is_edit_tool("edit"));
        assert!(is_subagent_tool("Agent") && is_subagent_tool("Task"));
        assert!(!is_subagent_tool("Bash"));
    }
}
