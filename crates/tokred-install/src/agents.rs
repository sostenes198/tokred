//! The tokred subagents: embedded templates and the install decision logic.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// A subagent template embedded in the binary.
#[derive(Debug, Clone, Copy)]
pub struct AgentTemplate {
    /// File name inside `<claude>/agents/` (always prefixed with `tokred-`).
    pub file_name: &'static str,
    /// Template text; `{{version}}` is replaced by the tokred version.
    pub content: &'static str,
}

macro_rules! template {
    ($name:literal) => {
        AgentTemplate {
            file_name: concat!($name, ".md"),
            content: include_str!(concat!("../templates/", $name, ".md")),
        }
    };
}

/// Every subagent tokred installs.
pub const TEMPLATES: [AgentTemplate; 6] = [
    template!("tokred-architect"),
    template!("tokred-planner"),
    template!("tokred-dev"),
    template!("tokred-reviewer"),
    template!("tokred-explorer"),
    template!("tokred-summarizer"),
];

/// Prefix of every file tokred owns inside `<claude>/agents/`.
pub const FILE_PREFIX: &str = "tokred-";

/// Renders a template for `version` (e.g. `0.1.0`).
#[must_use]
pub fn render(template: &AgentTemplate, version: &str) -> String {
    template
        .content
        .replace("{{version}}", &format!("v{version}"))
}

/// blake3 hash (hex) of `content`.
#[must_use]
pub fn content_hash(content: &[u8]) -> String {
    blake3::hash(content).to_hex().to_string()
}

/// Folder holding the agents inside the Claude Code folder.
#[must_use]
pub fn agents_dir(claude_dir: &Path) -> PathBuf {
    claude_dir.join("agents")
}

/// Path of the sibling file written when the user edited the installed agent.
#[must_use]
pub fn conflict_path(agent_path: &Path) -> PathBuf {
    let mut name = agent_path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(".new");
    agent_path.with_file_name(name)
}

/// What `configure` does with one agent file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAction {
    /// The file does not exist: create it.
    Create,
    /// The file is an untouched older install: overwrite it.
    Update,
    /// The file already has the desired content.
    Unchanged,
    /// The user edited the file: leave it and write `<file>.new` next to it.
    Conflict,
    /// The user edited the file but `--force` was given: overwrite it.
    Overwrite,
}

/// Decides what to do with an agent file.
///
/// * `desired_hash` – hash of the content to install;
/// * `current_hash` – hash of the file on disk, `None` if absent;
/// * `manifest_hash` – hash recorded when tokred last installed it, `None` if unknown.
#[must_use]
pub fn plan_agent(
    desired_hash: &str,
    current_hash: Option<&str>,
    manifest_hash: Option<&str>,
    force: bool,
) -> AgentAction {
    match current_hash {
        None => AgentAction::Create,
        Some(current) if current == desired_hash => AgentAction::Unchanged,
        Some(current) if manifest_hash == Some(current) => AgentAction::Update,
        Some(_) if force => AgentAction::Overwrite,
        Some(_) => AgentAction::Conflict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_have_expected_shape() {
        let expected = [
            ("tokred-architect.md", "opus"),
            ("tokred-planner.md", "opus"),
            ("tokred-dev.md", "sonnet"),
            ("tokred-reviewer.md", "sonnet"),
            ("tokred-explorer.md", "haiku"),
            ("tokred-summarizer.md", "haiku"),
        ];
        for (template, (file, model)) in TEMPLATES.iter().zip(expected) {
            assert_eq!(template.file_name, file);
            assert!(template.file_name.starts_with(FILE_PREFIX));
            let rendered = render(template, "1.2.3");
            let mut lines = rendered.lines();
            assert_eq!(lines.next(), Some("---"));
            assert!(rendered.contains(&format!("\nmodel: {model}\n")), "{file}");
            assert!(rendered.contains(&format!("name: {}\n", file.trim_end_matches(".md"))));
            // The managed-by marker is the first body line, right after the frontmatter.
            let after_frontmatter = rendered.splitn(3, "---\n").nth(2).unwrap();
            assert_eq!(
                after_frontmatter.lines().next(),
                Some("<!-- managed-by: tokred v1.2.3 -->")
            );
            assert!(rendered.contains("RESULT:") && rendered.contains("PENDING:"));
            assert!(!rendered.contains("{{"));
        }
    }

    #[test]
    fn read_only_agents_cannot_edit() {
        for template in TEMPLATES {
            let read_only = ["architect", "planner", "explorer", "summarizer"]
                .iter()
                .any(|role| template.file_name.contains(role));
            if read_only {
                let tools = template
                    .content
                    .lines()
                    .find(|l| l.starts_with("tools:"))
                    .unwrap();
                assert!(
                    !tools.contains("Edit") && !tools.contains("Write") && !tools.contains("Bash")
                );
            }
        }
    }

    #[test]
    fn plan_table() {
        use AgentAction::*;
        assert_eq!(plan_agent("n", None, None, false), Create);
        assert_eq!(plan_agent("n", None, Some("old"), false), Create);
        assert_eq!(plan_agent("n", Some("n"), Some("old"), false), Unchanged);
        assert_eq!(plan_agent("n", Some("old"), Some("old"), false), Update);
        assert_eq!(
            plan_agent("n", Some("edited"), Some("old"), false),
            Conflict
        );
        assert_eq!(plan_agent("n", Some("edited"), None, false), Conflict);
        assert_eq!(
            plan_agent("n", Some("edited"), Some("old"), true),
            Overwrite
        );
    }

    #[test]
    fn conflict_path_appends_new() {
        assert_eq!(
            conflict_path(Path::new("/c/agents/tokred-dev.md")),
            PathBuf::from("/c/agents/tokred-dev.md.new")
        );
    }
}
