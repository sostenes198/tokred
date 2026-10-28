//! `tokred audit`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokred_audit::fs::{StdDirLister, TranscriptScanner};
use tokred_audit::report::AuditReport;
use tokred_audit::{AuditRequest, Ports, audit};
use tokred_install::StdFileSystem;

use super::{SystemClock, select_claude_dir};
use crate::env::Env;
use crate::io::Io;
use crate::term::Term;
use crate::view::render_audit;

/// The project to audit: the given path, else the current folder, made absolute and resolved
/// (a symlinked path such as `/tmp` would otherwise miss the project's local MCP servers).
fn resolve_project(project: Option<PathBuf>) -> Result<PathBuf> {
    let path = match project {
        Some(path) => path,
        None => std::env::current_dir().context("could not read the current folder")?,
    };
    Ok(path.canonicalize().unwrap_or(path))
}

/// Audits `project` against the Claude Code folder.
pub fn build_report(env: &Env, claude_dir: &Path, project: PathBuf) -> AuditReport {
    let request = AuditRequest::new(claude_dir.to_owned(), env.home_dir.clone(), Some(project));
    let scanner = TranscriptScanner::new(claude_dir.join("projects"));
    let ports = Ports {
        fs: &StdFileSystem,
        dirs: &StdDirLister,
        usage: &scanner,
        clock: &SystemClock,
    };
    audit(&ports, &request)
}

/// The project to audit when none is given: the current folder.
pub fn default_project() -> Result<PathBuf> {
    resolve_project(None)
}

pub fn run(
    env: &Env,
    io: &mut Io,
    dir_flag: Option<PathBuf>,
    project: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    let claude_dir = select_claude_dir(&StdFileSystem, env, io, dir_flag, false)?;
    let report = build_report(env, &claude_dir, resolve_project(project)?);

    if json {
        let text = serde_json::to_string_pretty(&report)?;
        writeln!(io.out(), "{text}")?;
    } else {
        let shown = render_audit(Term::new(io.color()), &report);
        write!(io.out(), "{shown}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted;

    fn seeded() -> Sandbox {
        let sb = Sandbox::new();
        std::fs::write(sb.claude.join("CLAUDE.md"), "x".repeat(4_000)).unwrap();
        std::fs::create_dir_all(sb.claude.join("agents")).unwrap();
        std::fs::write(
            sb.claude.join("agents/helper.md"),
            "---\nname: helper\ndescription: d\n---\nbody",
        )
        .unwrap();
        let project = sb.path("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join(".mcp.json"),
            r#"{"mcpServers":{"idle":{"command":"x"}}}"#,
        )
        .unwrap();
        sb
    }

    #[test]
    fn the_table_report_lists_items_and_mcp_servers() {
        let sb = seeded();
        let (mut io, out) = scripted(false, "");
        run(
            &sb.env,
            &mut io,
            Some(sb.claude.clone()),
            Some(sb.path("project")),
            false,
        )
        .unwrap();
        let text = out.text();
        for expected in [
            "tokred Context Audit",
            "Fixed context:",
            "Heaviest Items",
            "CLAUDE.md",
            "helper",
            "MCP Servers",
            "idle",
            "unused",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
    }

    #[test]
    fn json_output_is_the_raw_report() {
        let sb = seeded();
        let (mut io, out) = scripted(false, "");
        run(
            &sb.env,
            &mut io,
            Some(sb.claude.clone()),
            Some(sb.path("project")),
            true,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&out.text()).unwrap();
        assert!(
            value["items"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
        assert!(value["summary"]["total_estimated_tokens"].as_u64().unwrap() > 0);
    }

    #[test]
    fn the_project_defaults_to_the_current_folder_and_resolves_symlinks() {
        let here = resolve_project(None).unwrap();
        assert!(here.is_absolute());
        let sb = Sandbox::new();
        assert_eq!(
            resolve_project(Some(sb.path("does-not-exist"))).unwrap(),
            sb.path("does-not-exist")
        );
    }
}
