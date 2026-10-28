//! `tokred configure`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokred_core::meta::MetaRepo;
use tokred_core::{FileSystem, ManifestRepo};
use tokred_install::agents::AgentAction;
use tokred_install::settings::{ChangeKind, hook_entries};
use tokred_install::{
    ConfigureInput, ConfigurePlan, ConfigureReport, StdFileSystem, apply_configure, plan_configure,
};

use super::data::FIRST_CONFIGURED_AT;
use super::{SystemClock, existing_store, open_store, select_claude_dir, show};
use crate::binary::{self, BinaryPath};
use crate::env::Env;
use crate::io::Io;
use crate::logging;

pub fn run(
    env: &Env,
    io: &mut Io,
    dir_flag: Option<PathBuf>,
    yes: bool,
    force: bool,
) -> Result<()> {
    let fs = StdFileSystem;
    let claude_dir = select_claude_dir(&fs, env, io, dir_flag, true)?;
    let binary = binary::find()?;
    let version = env!("CARGO_PKG_VERSION");

    let store = existing_store(env)?;
    let input = ConfigureInput {
        claude_dir: &claude_dir,
        binary: &binary.path,
        version,
        force,
    };
    let manifest = store.as_ref().map(|s| s as &dyn ManifestRepo);
    let plan = plan_configure(&fs, manifest, &input)?;

    let rtk = binary::rtk_on_path()
        || plan
            .original_settings
            .as_deref()
            .is_some_and(|text| text.contains("rtk hook"));
    let shown = render_plan(&claude_dir, &binary, rtk, &plan);
    write!(io.out(), "{shown}")?;

    if !yes && !confirmed(io)? {
        writeln!(io.out(), "Nothing was changed.")?;
        return Ok(());
    }

    fs.create_dir_all(env.data.root())?;
    logging::init(&env.data.logs_dir());
    let config_path = env.data.config_path();
    let existing = fs.read_to_string(&config_path)?.unwrap_or_default();
    let merged = tokred_core::config::merge_config(&existing, Some(&claude_dir))
        .with_context(|| format!("could not update {}", config_path.display()))?;
    if merged != existing {
        fs.write_atomic(&config_path, merged.as_bytes())?;
    }

    let store = match store {
        Some(store) => store,
        None => open_store(&env.data.db_path())?,
    };
    // The database may have been created just now, so recompute against it.
    let plan = plan_configure(&fs, Some(&store as &dyn ManifestRepo), &input)?;
    let report = apply_configure(&fs, &store, &SystemClock, &plan)?;
    tokred_update::mark_configured(&store, version)?;
    if store.meta_get(FIRST_CONFIGURED_AT)?.is_none() {
        let now = tokred_core::Clock::now(&SystemClock).0.to_string();
        store.meta_set(FIRST_CONFIGURED_AT, &now)?;
    }
    tracing::info!(claude_dir = %claude_dir.display(), "configured");

    let summary = render_summary(&plan, &report);
    write!(io.out(), "{summary}")?;
    Ok(())
}

/// Asks for confirmation; refuses to guess when nobody can answer.
fn confirmed(io: &mut Io) -> Result<bool> {
    if !io.interactive() {
        bail!("not running in a terminal; pass --yes to apply the plan above");
    }
    let answer = io.ask("Apply these changes? [y/N] ")?.unwrap_or_default();
    Ok(matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes"))
}

/// The plan shown before anything is applied. (Writing to a `String` cannot fail.)
fn render_plan(claude_dir: &Path, binary: &BinaryPath, rtk: bool, plan: &ConfigurePlan) -> String {
    let mut out = String::from("tokred configure\n");
    let _ = writeln!(out, "  Claude Code folder: {}", show(claude_dir));
    if binary.stable {
        let _ = writeln!(out, "  Binary: {}", binary.path);
    } else {
        let _ = writeln!(
            out,
            "  Binary: {} (not found on PATH; hooks will point to this exact file)",
            binary.path
        );
    }
    if rtk {
        out.push_str("  rtk detected: tokred and rtk will coexist (rtk keeps the Bash matcher)\n");
    }

    let _ = writeln!(out, "\nHooks in {}:", show(&plan.settings_path));
    let desired = hook_entries(&binary.path);
    for change in &plan.hook_changes {
        let symbol = match change.kind {
            ChangeKind::Added => '+',
            ChangeKind::Updated => '~',
            ChangeKind::Unchanged => '=',
            ChangeKind::Removed => '-',
        };
        let matcher = desired
            .iter()
            .find(|d| d.hook_arg() == Some(change.hook.as_str()))
            .and_then(|d| d.matcher.as_deref())
            .map(|m| format!(" [{m}]"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  {symbol} {:<14} {}{matcher}",
            change.hook, change.event
        );
    }

    out.push_str("\nAgents in agents/:\n");
    for agent in &plan.agents {
        let text = match agent.action {
            AgentAction::Create => "+ create",
            AgentAction::Update => "~ update",
            AgentAction::Unchanged => "= up to date",
            AgentAction::Conflict => "! you edited it: will write a .new file next to it",
            AgentAction::Overwrite => "~ overwrite (--force)",
        };
        let _ = writeln!(out, "  {:<24} {text}", agent.file_name);
    }
    if plan.new_settings.is_some() && plan.original_settings.is_some() {
        let _ = writeln!(
            out,
            "\nA backup will be saved as {}",
            show(&plan.backup_path)
        );
    }
    out.push('\n');
    out
}

/// What was done, shown after the plan has been applied.
fn render_summary(plan: &ConfigurePlan, report: &ConfigureReport) -> String {
    let mut out = String::from("\n");
    if report.settings_written {
        let _ = writeln!(out, "Updated {}", show(&plan.settings_path));
        if report.backup_written {
            let _ = writeln!(
                out,
                "Backup of the previous file: {}",
                show(&plan.backup_path)
            );
        }
    } else {
        out.push_str("settings.json already up to date.\n");
    }
    for (file, action) in &report.agents {
        if *action == AgentAction::Conflict {
            let _ = writeln!(
                out,
                "You edited {file}: kept your version and wrote {file}.new next to it."
            );
        }
    }
    out.push_str("\nNext steps:\n  tokred doctor    check the installation\n");
    out
}

#[cfg(test)]
mod tests {
    use tokred_core::{ManifestEntry, ManifestRepo, Timestamp};
    use tokred_install::agents::content_hash;
    use tokred_store::Store;

    use super::super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted;

    fn configure(sb: &Sandbox, yes: bool, force: bool, input: &str) -> (Result<()>, String) {
        let (mut io, out) = scripted(!yes, input);
        let result = run(&sb.env, &mut io, Some(sb.claude.clone()), yes, force);
        (result, out.text())
    }

    /// Records `content` as the version tokred installed, so the next run sees an old install.
    fn pretend_installed(sb: &Sandbox, name: &str, content: &str) {
        std::fs::write(sb.agent(name), content).unwrap();
        let store = Store::open(&sb.env.data.db_path()).unwrap();
        store
            .upsert(&ManifestEntry {
                path: sb.agent(name),
                hash: content_hash(content.as_bytes()),
                version: "0.0.1".to_owned(),
                installed_at: Timestamp(1),
            })
            .unwrap();
    }

    #[test]
    fn the_plan_text_does_not_depend_on_what_is_installed_on_this_machine() {
        let sb = Sandbox::new();
        sb.write_settings("{}");
        let input = ConfigureInput {
            claude_dir: &sb.claude,
            binary: "/opt/bin/tokred",
            version: "1.0.0",
            force: false,
        };
        let plan = plan_configure(&StdFileSystem, None, &input).unwrap();
        let on_path = BinaryPath {
            path: "/opt/bin/tokred".to_owned(),
            stable: true,
        };
        let off_path = BinaryPath {
            path: "/tmp/tokred".to_owned(),
            stable: false,
        };

        let text = render_plan(&sb.claude, &on_path, false, &plan);
        assert!(text.contains("  Binary: /opt/bin/tokred\n"), "{text}");
        assert!(!text.contains("not found on PATH"));
        assert!(!text.contains("rtk detected"));

        let text = render_plan(&sb.claude, &off_path, true, &plan);
        assert!(text.contains("/tmp/tokred (not found on PATH"), "{text}");
        assert!(text.contains("rtk detected"));
    }

    #[test]
    fn confirming_with_y_applies_the_plan() {
        let sb = Sandbox::new();
        sb.write_settings(r#"{"model": "opus"}"#);
        let (result, out) = configure(&sb, false, false, "y\n");
        result.unwrap();
        assert!(out.contains("Apply these changes? [y/N] "));
        assert!(out.contains("Updated "));
        assert!(out.contains("Backup of the previous file"));
        assert!(out.contains("A backup will be saved as"));
        assert!(sb.agent("tokred-dev.md").exists());
        assert!(sb.env.data.db_path().exists());
    }

    #[test]
    fn answering_no_or_nothing_changes_nothing() {
        for answer in ["n\n", "\n", "maybe\n", ""] {
            let sb = Sandbox::new();
            sb.write_settings("{}");
            let (result, out) = configure(&sb, false, false, answer);
            result.unwrap();
            assert!(out.contains("Nothing was changed."), "{answer:?}");
            assert!(!sb.env.data.root().exists());
            assert_eq!(std::fs::read_to_string(sb.settings()).unwrap(), "{}");
        }
    }

    #[test]
    fn without_a_terminal_and_without_yes_it_refuses() {
        let sb = Sandbox::new();
        let (mut io, _) = scripted(false, "y\n");
        let err = run(&sb.env, &mut io, Some(sb.claude.clone()), false, false).unwrap_err();
        assert!(err.to_string().contains("--yes"));
    }

    #[test]
    fn a_second_run_reports_nothing_to_do() {
        let sb = Sandbox::new();
        configure(&sb, true, false, "").0.unwrap();
        let (result, out) = configure(&sb, true, false, "");
        result.unwrap();
        assert!(out.contains("settings.json already up to date."));
        assert!(out.contains("= up to date"));
        assert!(!out.contains("A backup will be saved"));
    }

    #[test]
    fn the_plan_marks_updated_and_removed_hooks_and_detects_rtk_in_settings() {
        let sb = Sandbox::new();
        sb.write_settings(
            r#"{"hooks": {
                "PreToolUse": [
                    {"matcher": "Bash", "hooks": [{"type": "command", "command": "rtk hook claude"}]},
                    {"matcher": "Read", "hooks": [{"type": "command", "command": "\"/old/tokred\" hook guard-read"}]}
                ],
                "Stop": [{"hooks": [{"type": "command", "command": "\"/old/tokred\" hook retired"}]}]
            }}"#,
        );
        let (result, out) = configure(&sb, true, false, "");
        result.unwrap();
        assert!(out.contains("rtk detected"));
        assert!(out.contains("~ guard-read"), "{out}");
        assert!(out.contains("- retired"), "{out}");
        assert!(out.contains("+ route"));
    }

    #[test]
    fn an_old_untouched_agent_is_updated() {
        let sb = Sandbox::new();
        configure(&sb, true, false, "").0.unwrap();
        pretend_installed(&sb, "tokred-dev.md", "an older version");

        let (result, out) = configure(&sb, true, false, "");
        result.unwrap();
        assert!(out.contains("~ update"), "{out}");
        assert!(
            std::fs::read_to_string(sb.agent("tokred-dev.md"))
                .unwrap()
                .contains("managed-by: tokred v")
        );
    }

    #[test]
    fn an_edited_agent_is_kept_unless_forced() {
        let sb = Sandbox::new();
        configure(&sb, true, false, "").0.unwrap();
        std::fs::write(sb.agent("tokred-dev.md"), "my edits").unwrap();

        let (result, out) = configure(&sb, true, false, "");
        result.unwrap();
        assert!(out.contains("! you edited it"));
        assert!(out.contains("You edited tokred-dev.md: kept your version"));
        assert_eq!(
            std::fs::read_to_string(sb.agent("tokred-dev.md")).unwrap(),
            "my edits"
        );

        let (result, out) = configure(&sb, true, true, "");
        result.unwrap();
        assert!(out.contains("~ overwrite (--force)"));
        assert!(
            std::fs::read_to_string(sb.agent("tokred-dev.md"))
                .unwrap()
                .contains("managed-by")
        );
    }

    #[test]
    fn invalid_settings_json_aborts_before_anything_is_created() {
        let sb = Sandbox::new();
        sb.write_settings("{ nope");
        let (result, _) = configure(&sb, true, false, "");
        assert!(result.unwrap_err().to_string().contains("not valid JSON"));
        assert!(!sb.env.data.root().exists());
    }

    #[test]
    fn an_unreadable_config_aborts_after_the_plan() {
        let sb = Sandbox::new();
        std::fs::create_dir_all(sb.env.data.root()).unwrap();
        std::fs::write(sb.env.data.config_path(), "modes = 3\n").unwrap();
        let (result, _) = configure(&sb, true, false, "");
        assert!(result.unwrap_err().to_string().contains("could not update"));
    }

    #[test]
    fn a_database_that_cannot_be_created_is_reported() {
        let sb = Sandbox::new();
        // A directory where the database file should be.
        std::fs::create_dir_all(sb.env.data.db_path()).unwrap();
        let (result, _) = configure(&sb, true, false, "");
        assert!(result.is_err());
    }
}
