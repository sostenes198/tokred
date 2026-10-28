//! The hook events tokred registers with Claude Code.

/// A tokred hook, invoked as `tokred hook <name>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookEvent {
    /// Suggests a subagent for the user prompt (`UserPromptSubmit`).
    Route,
    /// Checks the chosen subagent against the classification (`PreToolUse`).
    Enforce,
    /// Blocks useless file reads (`PreToolUse`).
    GuardRead,
    /// Blocks overly broad searches (`PreToolUse`).
    GuardSearch,
    /// Warns when the context grows too large (`Stop`).
    SessionCheck,
    /// Registers the session (`SessionStart`).
    SessionStart,
}

impl HookEvent {
    /// Every hook, in registration order.
    pub const ALL: [Self; 6] = [
        Self::Route,
        Self::Enforce,
        Self::GuardRead,
        Self::GuardSearch,
        Self::SessionCheck,
        Self::SessionStart,
    ];

    /// Argument of `tokred hook <arg>`.
    #[must_use]
    pub const fn arg(self) -> &'static str {
        match self {
            Self::Route => "route",
            Self::Enforce => "enforce",
            Self::GuardRead => "guard-read",
            Self::GuardSearch => "guard-search",
            Self::SessionCheck => "session-check",
            Self::SessionStart => "session-start",
        }
    }

    /// Parses the argument of `tokred hook <arg>`.
    #[must_use]
    pub fn from_arg(arg: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.arg() == arg)
    }

    /// Name of the Claude Code event the hook is registered under in `settings.json`.
    #[must_use]
    pub const fn claude_event(self) -> &'static str {
        match self {
            Self::Route => "UserPromptSubmit",
            Self::Enforce | Self::GuardRead | Self::GuardSearch => "PreToolUse",
            Self::SessionCheck => "Stop",
            Self::SessionStart => "SessionStart",
        }
    }

    /// Tool matcher of the settings group, when the event supports one.
    ///
    /// The subagent tool is named `Agent` (formerly `Task`), so both names are matched.
    #[must_use]
    pub const fn matcher(self) -> Option<&'static str> {
        match self {
            Self::Enforce => Some("Agent|Task"),
            Self::GuardRead => Some("Read"),
            Self::GuardSearch => Some("Grep|Glob"),
            Self::Route | Self::SessionCheck | Self::SessionStart => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_round_trip() {
        for event in HookEvent::ALL {
            assert_eq!(HookEvent::from_arg(event.arg()), Some(event));
        }
        assert_eq!(HookEvent::from_arg("bash"), None);
    }

    #[test]
    fn no_hook_matches_bash() {
        for event in HookEvent::ALL {
            assert!(!event.matcher().is_some_and(|m| m.contains("Bash")));
        }
    }
}
