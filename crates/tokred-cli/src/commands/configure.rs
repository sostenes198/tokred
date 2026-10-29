//! `tokred configure`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokred_core::meta::MetaRepo;
use tokred_core::{Config, FileSystem, ManifestRepo};
use tokred_install::agents::AgentAction;
use tokred_install::settings::{ChangeKind, hook_entries};
use tokred_install::{
    ConfigureInput, ConfigurePlan, ConfigureReport, StdFileSystem, apply_configure, plan_configure,
};

use super::advice::set_key;
use super::data::FIRST_CONFIGURED_AT;
use super::port::{self, PortState};
use super::{SystemClock, existing_store, open_store, proxy, select_claude_dir, show};
use crate::binary::{self, BinaryPath};
use crate::env::Env;
use crate::io::Io;
use crate::logging;

/// What the user chose for the proxy through flags or environment variables; `None` means
/// "nothing said": the existing configuration (or the built-in default) stays, and in a
/// terminal the user is asked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProxyChoices {
    /// `--proxy-port` or `TOKRED_PORT`.
    pub port: Option<u16>,
    /// `--record-content` / `--no-record-content` or `TOKRED_RECORD_CONTENT`.
    pub record_content: Option<bool>,
}

/// Everything `configure` is told besides the environment and the terminal.
pub struct Options<'a> {
    /// `--claude-dir`.
    pub dir_flag: Option<PathBuf>,
    /// `--yes`: no questions at all.
    pub yes: bool,
    /// `--force`.
    pub force: bool,
    /// The proxy answers given up front.
    pub choices: ProxyChoices,
    /// What listens on a port (seam: unit tests never touch real ports).
    pub probe: &'a dyn Fn(u16) -> PortState,
}

/// `configure` with the real port probe and no proxy choices (what the other commands use).
pub fn run(
    env: &Env,
    io: &mut Io,
    dir_flag: Option<PathBuf>,
    yes: bool,
    force: bool,
) -> Result<()> {
    let probe = |port| port::probe(&proxy::files(env), port);
    run_with(
        env,
        io,
        &Options {
            dir_flag,
            yes,
            force,
            choices: ProxyChoices::default(),
            probe: &probe,
        },
    )
}

pub fn run_with(env: &Env, io: &mut Io, options: &Options<'_>) -> Result<()> {
    let fs = StdFileSystem;
    let claude_dir = select_claude_dir(&fs, env, io, options.dir_flag.clone(), true)?;
    let binary = binary::find()?;
    let version = env!("CARGO_PKG_VERSION");

    let store = existing_store(env)?;
    let input = ConfigureInput {
        claude_dir: &claude_dir,
        binary: &binary.path,
        version,
        force: options.force,
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

    if !options.yes && !confirmed(io)? {
        writeln!(io.out(), "Nothing was changed.")?;
        return Ok(());
    }

    fs.create_dir_all(env.data.root())?;
    logging::init(&env.data.logs_dir());
    let config_path = env.data.config_path();
    let existing = fs.read_to_string(&config_path)?.unwrap_or_default();
    let setup = choose_proxy_setup(io, options, &existing)?;
    let merged = merge_with_proxy_default(&existing, &claude_dir)
        .map_err(anyhow::Error::from)
        .and_then(|merged| apply_proxy_setup(&merged, setup))
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

    let proxy_config = Config::from_toml_str(&merged)
        .map(|config| config.proxy)
        .unwrap_or_default();
    let summary = render_summary(&plan, &report, &proxy_config);
    write!(io.out(), "{summary}")?;
    Ok(())
}

/// Merges the defaults into `tokred.toml` and, when the user had no `[proxy] enabled` value yet,
/// turns the proxy features on (an explicit value, either way, is kept).
fn merge_with_proxy_default(
    existing: &str,
    claude_dir: &Path,
) -> Result<String, tokred_core::ConfigError> {
    let had_value = tokred_core::config::proxy_enabled_is_set(existing);
    let merged = tokred_core::config::merge_config(existing, Some(claude_dir))?;
    if had_value {
        return Ok(merged);
    }
    tokred_core::config::set_config_value(&merged, "proxy", "enabled", toml::Value::Boolean(true))
}

/// What `configure` will write for the proxy; `None` leaves the value in `tokred.toml` alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProxySetup {
    port: Option<u16>,
    record_content: Option<bool>,
}

/// Decides the proxy port and content recording.
///
/// A flag or environment variable wins and is not asked about. Otherwise, with a person at the
/// terminal and no `--yes`, the user is asked (the current value is the default). Without one,
/// nothing is written, so an existing value survives the re-run that every upgrade does.
fn choose_proxy_setup(io: &mut Io, options: &Options<'_>, existing: &str) -> Result<ProxySetup> {
    let ask = !options.yes;
    let port = match options.choices.port {
        Some(port) => {
            warn_if_busy(io, options, port)?;
            Some(port)
        }
        None if ask => Some(ask_port(io, options.probe, current_port(existing))?),
        None => None,
    };
    let record_content = match options.choices.record_content {
        Some(record) => Some(record),
        None if ask => Some(ask_record_content(io, existing)?),
        None => None,
    };
    Ok(ProxySetup {
        port,
        record_content,
    })
}

/// The port the proxy is configured for now, else the built-in default.
fn current_port(existing: &str) -> u16 {
    Config::from_toml_str(existing)
        .ok()
        .and_then(|config| crate::daemon::port_of(&config.proxy.bind).ok())
        .unwrap_or(port::DEFAULT_PORT)
}

/// A port given by flag is accepted even when something listens there, but the user is told.
fn warn_if_busy(io: &mut Io, options: &Options<'_>, port: u16) -> Result<()> {
    if (options.probe)(port) == PortState::Busy {
        let warning = format!(
            "Port {port} is in use by another program: the proxy cannot start until it is free (or choose another with `tokred configure --proxy-port <port>`).\n"
        );
        write!(io.out(), "{warning}")?;
    }
    Ok(())
}

/// Asks for the proxy port until it is valid and free (or already ours).
fn ask_port(io: &mut Io, probe: &dyn Fn(u16) -> PortState, default: u16) -> Result<u16> {
    loop {
        let answer = io.ask(&format!("Port for the tokred proxy [{default}]: "))?;
        let at_end = answer.is_none();
        let typed = answer.unwrap_or_default();
        let parsed = if typed.is_empty() {
            Ok(default)
        } else {
            port::parse_port(&typed)
        };
        match parsed {
            Err(message) => writeln!(io.out(), "{message}")?,
            Ok(port) if probe(port) == PortState::Busy => {
                let message = format!("Port {port} is in use by another program. Choose another.");
                writeln!(io.out(), "{message}")?;
                if at_end {
                    bail!("no free port was given for the proxy");
                }
            }
            Ok(port) => return Ok(port),
        }
    }
}

/// Explains and asks whether the dashboard may show what was sent and returned. The default
/// is the value already in `tokred.toml`, else yes.
fn ask_record_content(io: &mut Io, existing: &str) -> Result<bool> {
    let default = explicit_log_bodies(existing).unwrap_or(true);
    let explanation = "\nThe dashboard can show what was sent to the model and what came back.\n\
        That needs tokred to keep the conversation text in its own database on this machine;\n\
        headers and credentials are never recorded, and nothing leaves this computer.\n";
    write!(io.out(), "{explanation}")?;
    let hint = if default { "[Y/n]" } else { "[y/N]" };
    let question = format!("Record conversation content so the dashboard can show it? {hint} ");
    let answer = io.ask(&question)?.unwrap_or_default().to_ascii_lowercase();
    Ok(match answer.as_str() {
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default,
    })
}

/// `[proxy] log_bodies` as written in the file, if it is there.
fn explicit_log_bodies(existing: &str) -> Option<bool> {
    toml::from_str::<toml::Table>(existing)
        .ok()?
        .get("proxy")?
        .get("log_bodies")?
        .as_bool()
}

/// Writes the choices into the `tokred.toml` text.
fn apply_proxy_setup(text: &str, setup: ProxySetup) -> Result<String> {
    let mut text = text.to_owned();
    if let Some(port) = setup.port {
        let current = Config::from_toml_str(&text)?.proxy.bind;
        let bind = toml::Value::String(port::with_port(&current, port));
        text = set_key(&text, "proxy", "bind", bind)?;
    }
    if let Some(record) = setup.record_content {
        text = set_key(&text, "proxy", "log_bodies", toml::Value::Boolean(record))?;
    }
    Ok(text)
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
fn render_summary(
    plan: &ConfigurePlan,
    report: &ConfigureReport,
    proxy: &tokred_core::config::ProxyConfig,
) -> String {
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
    let content = if proxy.log_bodies {
        "recorded for the dashboard"
    } else {
        "not recorded"
    };
    let _ = writeln!(out, "\nProxy address: {}", proxy.bind);
    let _ = writeln!(out, "Conversation content: {content}");
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
        configure_with(sb, yes, force, input, ProxyChoices::default(), &|_| {
            PortState::Free
        })
    }

    /// `configure` with proxy choices and a fake port probe (tests never touch real ports).
    fn configure_with(
        sb: &Sandbox,
        yes: bool,
        force: bool,
        input: &str,
        choices: ProxyChoices,
        probe: &dyn Fn(u16) -> PortState,
    ) -> (Result<()>, String) {
        let (mut io, out) = scripted(!yes, input);
        let options = Options {
            dir_flag: Some(sb.claude.clone()),
            yes,
            force,
            choices,
            probe,
        };
        let result = run_with(&sb.env, &mut io, &options);
        (result, out.text())
    }

    fn saved(sb: &Sandbox) -> Config {
        let text = std::fs::read_to_string(sb.env.data.config_path()).unwrap();
        Config::from_toml_str(&text).unwrap()
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
    fn a_fresh_configure_enables_the_proxy_features_but_an_explicit_value_is_kept() {
        let dir = Path::new("/c");
        let fresh = merge_with_proxy_default("", dir).unwrap();
        let enabled = |text: &str| {
            tokred_core::Config::from_toml_str(text)
                .unwrap()
                .proxy
                .enabled
        };
        assert!(enabled(&fresh));
        // Running it again does not change anything.
        assert_eq!(merge_with_proxy_default(&fresh, dir).unwrap(), fresh);

        // A file without the key (other [proxy] settings) gets it; an explicit false stays.
        assert!(enabled(
            &merge_with_proxy_default("[proxy]\nbind = \"127.0.0.1:1\"\n", dir).unwrap()
        ));
        let off = "[proxy]\nenabled = false\n";
        assert!(!enabled(&merge_with_proxy_default(off, dir).unwrap()));
        assert!(merge_with_proxy_default("[[[", dir).is_err());
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

    const ANSWERS_DEFAULT: &str = "y\n\n\n";

    #[test]
    fn a_person_is_asked_for_the_port_and_for_recording_with_these_defaults() {
        let sb = Sandbox::new();
        let (result, out) = configure(&sb, false, false, ANSWERS_DEFAULT);
        result.unwrap();
        assert!(out.contains("Port for the tokred proxy [1029]: "), "{out}");
        assert!(out.contains("nothing leaves this computer"), "{out}");
        assert!(out.contains("show it? [Y/n] "), "{out}");
        assert!(out.contains("Proxy address: 127.0.0.1:1029"), "{out}");
        assert!(out.contains("recorded for the dashboard"), "{out}");
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "127.0.0.1:1029");
        assert!(config.proxy.log_bodies);
    }

    #[test]
    fn the_answers_are_written_to_the_config() {
        let sb = Sandbox::new();
        let (result, out) = configure(&sb, false, false, "y\n2000\nn\n");
        result.unwrap();
        assert!(out.contains("Conversation content: not recorded"), "{out}");
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "127.0.0.1:2000");
        assert!(!config.proxy.log_bodies);

        // Asked again, the saved values are the defaults: pressing Enter keeps them.
        let (result, out) = configure(&sb, false, false, "y\n\n\n");
        result.unwrap();
        assert!(out.contains("Port for the tokred proxy [2000]: "), "{out}");
        assert!(out.contains("show it? [y/N] "), "{out}");
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "127.0.0.1:2000");
        assert!(!config.proxy.log_bodies);

        let (result, _) = configure(&sb, false, false, "y\n\nYES\n");
        result.unwrap();
        assert!(saved(&sb).proxy.log_bodies);
    }

    #[test]
    fn a_bad_or_busy_port_is_asked_again() {
        let sb = Sandbox::new();
        let busy = |port: u16| {
            if port == 3000 {
                PortState::Busy
            } else {
                PortState::Free
            }
        };
        let choices = ProxyChoices::default();
        let input = "y\nabc\n80\n3000\n3001\nn\n";
        let (result, out) = configure_with(&sb, false, false, input, choices, &busy);
        result.unwrap();
        assert!(out.contains("`abc` is not a port number"), "{out}");
        assert!(out.contains("`80` is not a port number"), "{out}");
        assert!(out.contains("Port 3000 is in use by another program. Choose another."));
        assert_eq!(saved(&sb).proxy.bind, "127.0.0.1:3001");
    }

    #[test]
    fn a_port_already_served_by_this_proxy_is_accepted() {
        let sb = Sandbox::new();
        let ours = |_| PortState::Ours;
        let (result, _) = configure_with(
            &sb,
            false,
            false,
            "y\n4000\nn\n",
            ProxyChoices::default(),
            &ours,
        );
        result.unwrap();
        assert_eq!(saved(&sb).proxy.bind, "127.0.0.1:4000");
    }

    #[test]
    fn a_busy_default_at_the_end_of_input_gives_up_instead_of_asking_forever() {
        let sb = Sandbox::new();
        let (result, _) =
            configure_with(&sb, false, false, "y\n", ProxyChoices::default(), &|_| {
                PortState::Busy
            });
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no free port was given")
        );
        assert!(!sb.env.data.config_path().exists(), "nothing was written");
    }

    #[test]
    fn flags_answer_for_the_user_and_a_busy_flag_port_only_warns() {
        let sb = Sandbox::new();
        let choices = ProxyChoices {
            port: Some(5000),
            record_content: Some(true),
        };
        let (result, out) = configure_with(&sb, false, false, "y\n", choices, &|_| PortState::Busy);
        result.unwrap();
        assert!(out.contains("Port 5000 is in use by another program: the proxy cannot start"));
        assert!(!out.contains("Port for the tokred proxy"), "{out}");
        assert!(!out.contains("Record conversation"), "{out}");
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "127.0.0.1:5000");
        assert!(config.proxy.log_bodies);
    }

    #[test]
    fn without_a_terminal_flags_apply_and_nothing_else_changes() {
        let sb = Sandbox::new();
        // First install, nothing said: the defaults (port 1029, content not recorded).
        let (result, out) = configure(&sb, true, false, "");
        result.unwrap();
        assert!(!out.contains("Port for the tokred proxy"), "{out}");
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "127.0.0.1:1029");
        assert!(!config.proxy.log_bodies);

        // The user chose to record: a quiet re-run (every upgrade does one) keeps it.
        let choices = ProxyChoices {
            port: Some(2500),
            record_content: Some(true),
        };
        configure_with(&sb, true, false, "", choices, &|_| PortState::Free)
            .0
            .unwrap();
        configure(&sb, true, false, "").0.unwrap();
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "127.0.0.1:2500");
        assert!(config.proxy.log_bodies);

        let off = ProxyChoices {
            port: None,
            record_content: Some(false),
        };
        configure_with(&sb, true, false, "", off, &|_| PortState::Free)
            .0
            .unwrap();
        assert!(!saved(&sb).proxy.log_bodies);
    }

    #[test]
    fn the_host_of_a_custom_bind_survives_a_port_change() {
        let sb = Sandbox::new();
        std::fs::create_dir_all(sb.env.data.root()).unwrap();
        std::fs::write(
            sb.env.data.config_path(),
            "[proxy]\nbind = \"0.0.0.0:7000\"\nlog_bodies = true\n",
        )
        .unwrap();
        let (result, out) = configure(&sb, false, false, "y\n\n\n");
        result.unwrap();
        assert!(out.contains("Port for the tokred proxy [7000]: "), "{out}");
        assert!(out.contains("show it? [Y/n] "), "{out}");
        let choices = ProxyChoices {
            port: Some(7100),
            record_content: None,
        };
        configure_with(&sb, true, false, "", choices, &|_| PortState::Free)
            .0
            .unwrap();
        assert_eq!(saved(&sb).proxy.bind, "0.0.0.0:7100");
    }

    #[test]
    fn an_unreadable_config_is_not_mistaken_for_a_choice() {
        assert_eq!(current_port("[[["), port::DEFAULT_PORT);
        assert_eq!(explicit_log_bodies("[[["), None);
        assert_eq!(explicit_log_bodies("[proxy]\nlog_bodies = 3\n"), None);
        assert_eq!(
            current_port("[proxy]\nbind = \"nonsense\"\n"),
            port::DEFAULT_PORT
        );
    }

    #[test]
    fn the_real_probe_is_what_run_uses() {
        // The convenience entry point wires the real probe; it only runs the prompts' probe
        // when someone is asked, so a --yes run touches no port at all.
        let sb = Sandbox::new();
        let (mut io, _) = scripted(false, "");
        run(&sb.env, &mut io, Some(sb.claude.clone()), true, false).unwrap();
        assert!(sb.env.data.config_path().exists());
    }
}
