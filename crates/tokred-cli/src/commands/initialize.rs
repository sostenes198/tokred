//! `tokred initialize`: reconciles Claude Code's `settings.json` with `[proxy] enabled`. A shell
//! wrapper for `claude` runs it before every launch, so it is fast when everything is already
//! right (one file read, one health probe, no database) and it never fails: every problem is a
//! warning on stderr and the exit code is 0. The same reconcile backs `tokred proxy
//! enable|disable` and the safety net of the `session-start` hook.
//!
//! The rules (the one thing reconciled is `env.ANTHROPIC_BASE_URL`):
//!
//! * disabled: tokred's own URL is removed; a value that is not tokred's is never touched, and
//!   nothing is started or stopped;
//! * enabled and the value is absent or tokred's: the proxy is started if it is not running and,
//!   once it answers, the URL is set. If it cannot be started the URL is removed again (a URL
//!   that points at a dead proxy would break Claude Code);
//! * enabled and the value is someone else's gateway: nothing is changed and nothing started.
//!
//! "Tokred's own URL" is the current proxy URL or any URL tokred wrote before (recorded in
//! `<data>/proxy/base_urls`), so a changed port is followed. The value exported in the shell
//! counts as the current value too.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::json;
use tokred_core::Config;
use tokred_install::StdFileSystem;
use tokred_install::base_url::{read_base_url, write_base_url};

use super::housekeeping;
use super::proxy::{files, spawn_proxy};
use crate::daemon::{base_url, healthy, port_of, wait_until};
use crate::env::Env;
use crate::io::Io;

/// Timeout of one health probe.
const PROBE: Duration = Duration::from_secs(1);
/// Timeout of the quick probe of the `session-start` hook.
const HOOK_PROBE: Duration = Duration::from_millis(200);

/// Where the reconcile ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Disabled: tokred's URL is not in the settings.
    Disabled,
    /// Enabled, the proxy answers and the settings point at it.
    Enabled,
    /// Enabled, but another gateway is configured: nothing was changed.
    OtherGateway,
    /// Enabled, but the proxy could not be started: the settings do not point at it.
    Unavailable,
    /// The settings could not be read: nothing was changed.
    Skipped,
}

impl State {
    /// Name used in `--json`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
            Self::OtherGateway => "other-gateway",
            Self::Unavailable => "unavailable",
            Self::Skipped => "skipped",
        }
    }
}

/// What a reconcile did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The outcome.
    pub state: State,
    /// What was changed, in plain words (shown by `--verbose`).
    pub actions: Vec<String>,
    /// Problems the user should know about (always shown, on stderr).
    pub warnings: Vec<String>,
    /// The proxy was started by this run.
    pub started: bool,
}

impl Report {
    fn new(state: State) -> Self {
        Self {
            state,
            actions: Vec::new(),
            warnings: Vec::new(),
            started: false,
        }
    }
}

impl Report {
    /// The warnings as the lines printed on stderr.
    #[must_use]
    pub fn warning_text(&self) -> String {
        self.warnings
            .iter()
            .fold(String::new(), |mut text, warning| {
                let _ = writeln!(text, "tokred: {warning}");
                text
            })
    }
}

/// What the reconcile needs from the machine.
pub struct Reconciler<'a> {
    /// `[proxy] enabled`.
    pub enabled: bool,
    /// The port of `[proxy] bind`.
    pub port: u16,
    /// `ANTHROPIC_BASE_URL` as exported in the shell.
    pub shell_value: Option<String>,
    /// Reads `env.ANTHROPIC_BASE_URL` from settings.json.
    pub read_url: &'a dyn Fn() -> Result<Option<String>>,
    /// Sets (`Some`) or removes (`None`) it; whether the file was written.
    pub write_url: &'a dyn Fn(Option<&str>) -> Result<bool>,
    /// The URLs tokred wrote before.
    pub known_urls: &'a dyn Fn() -> Vec<String>,
    /// Records a URL tokred is about to write.
    pub remember_url: &'a dyn Fn(&str),
    /// Whether the proxy answers on a port.
    pub healthy: &'a dyn Fn(u16) -> bool,
    /// Starts the proxy in the background.
    pub start_proxy: &'a dyn Fn() -> Result<()>,
    /// Waits for the proxy to answer on a port.
    pub wait_healthy: &'a dyn Fn(u16) -> bool,
}

/// Whether `value` is tokred's: the current proxy URL or one tokred wrote before.
fn is_tokred_url(value: &str, port: u16, known: &dyn Fn() -> Vec<String>) -> bool {
    let normalize = |url: &str| url.trim().trim_end_matches('/').to_owned();
    let value = normalize(value);
    value == base_url(port) || known().iter().any(|url| normalize(url) == value)
}

impl Reconciler<'_> {
    fn is_ours(&self, value: &str) -> bool {
        is_tokred_url(value, self.port, self.known_urls)
    }

    /// Reconciles the settings with `enabled`. Never fails.
    pub fn run(&self) -> Report {
        let current = match (self.read_url)() {
            Ok(current) => current,
            Err(error) => {
                let mut report = Report::new(State::Skipped);
                report.warnings.push(format!(
                    "could not read Claude Code's settings.json: {error:#}"
                ));
                return report;
            }
        };
        if self.enabled {
            self.enable(current.as_deref())
        } else {
            self.disable(current.as_deref())
        }
    }

    fn disable(&self, current: Option<&str>) -> Report {
        let mut report = Report::new(State::Disabled);
        match current {
            Some(value) if self.is_ours(value) => match (self.write_url)(None) {
                Ok(_) => report.actions.push(format!(
                    "removed ANTHROPIC_BASE_URL={value} from Claude Code's settings.json"
                )),
                Err(error) => report.warnings.push(format!(
                    "could not remove ANTHROPIC_BASE_URL from settings.json: {error:#}"
                )),
            },
            Some(value) => report.actions.push(format!(
                "left ANTHROPIC_BASE_URL={value} alone: it is not tokred's"
            )),
            None => {}
        }
        if let Some(shell) = self.shell_value.as_deref().filter(|v| self.is_ours(v)) {
            report.warnings.push(format!(
                "the tokred proxy is disabled but this shell exports ANTHROPIC_BASE_URL={shell}; unset it"
            ));
        }
        report
    }

    fn enable(&self, current: Option<&str>) -> Report {
        let foreign = current
            .into_iter()
            .chain(self.shell_value.as_deref())
            .find(|value| !self.is_ours(value));
        if let Some(value) = foreign {
            let mut report = Report::new(State::OtherGateway);
            report.warnings.push(format!(
                "the tokred proxy is enabled but ANTHROPIC_BASE_URL points to {value}, so tokred leaves it alone; \
                 to use the proxy remove that setting, or run `tokred proxy disable` to stop seeing this"
            ));
            return report;
        }

        let ours = base_url(self.port);
        let mut report = Report::new(State::Enabled);
        let mut why = String::new();
        let mut up = (self.healthy)(self.port);
        if !up {
            match (self.start_proxy)() {
                Ok(()) => {
                    up = (self.wait_healthy)(self.port);
                    report.started = up;
                    if !up {
                        why = format!("it did not answer on port {} in time", self.port);
                    }
                }
                Err(error) => why = format!("{error:#}"),
            }
        }
        if !up {
            report.state = State::Unavailable;
            if current.is_some() {
                match (self.write_url)(None) {
                    Ok(_) => report.actions.push(
                        "removed ANTHROPIC_BASE_URL from settings.json: the proxy is not running"
                            .to_owned(),
                    ),
                    Err(error) => report.warnings.push(format!(
                        "could not remove ANTHROPIC_BASE_URL from settings.json: {error:#}"
                    )),
                }
            }
            report.warnings.push(format!(
                "the tokred proxy is enabled but could not be started ({why}); Claude Code will not use it"
            ));
            return report;
        }
        if report.started {
            report
                .actions
                .push(format!("started the tokred proxy on {ours}"));
        }
        if current != Some(ours.as_str()) {
            (self.remember_url)(&ours);
            match (self.write_url)(Some(&ours)) {
                Ok(_) => report.actions.push(format!(
                    "set ANTHROPIC_BASE_URL={ours} in Claude Code's settings.json"
                )),
                Err(error) => report.warnings.push(format!(
                    "could not set ANTHROPIC_BASE_URL in settings.json: {error:#}"
                )),
            }
        }
        report
    }
}

/// `unconfigure`: removes `env.ANTHROPIC_BASE_URL` from `<claude_dir>/settings.json` when it is
/// tokred's (the current proxy URL or one tokred wrote), and returns the value removed. Any
/// other value is the user's and stays.
pub fn remove_own_url(env: &Env, config: &Config, claude_dir: &Path) -> Result<Option<String>> {
    let port = port_of(&config.proxy.bind).unwrap_or(0);
    let proxy_files = files(env);
    let fs = StdFileSystem;
    let Some(value) = read_base_url(&fs, claude_dir)? else {
        return Ok(None);
    };
    if !is_tokred_url(&value, port, &|| proxy_files.known_base_urls()) {
        return Ok(None);
    }
    write_base_url(&fs, claude_dir, None)?;
    Ok(Some(value))
}

/// The Claude Code folder whose settings this run reconciles: the one `claude` itself would use
/// (`$CLAUDE_CONFIG_DIR`), else the one saved by `configure`, else `~/.claude`.
#[must_use]
pub fn settings_dir(env: &Env, config: &Config) -> Option<PathBuf> {
    env.claude_config_dir
        .clone()
        .or_else(|| config.claude.dir.as_ref().map(PathBuf::from))
        .or_else(|| env.default_claude_dir.clone())
}

/// `ANTHROPIC_BASE_URL` as exported in this process (empty counts as unset).
#[must_use]
pub fn shell_base_url() -> Option<String> {
    std::env::var("ANTHROPIC_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Reconciles the real settings file of `env` with `enabled`, starting the proxy through `exe`
/// when needed and waiting up to `settle` for it.
pub fn reconcile(
    env: &Env,
    config: &Config,
    exe: &Path,
    enabled: bool,
    settle: Duration,
) -> Result<Report> {
    let port = port_of(&config.proxy.bind)?;
    let dir = settings_dir(env, config).context("could not determine the Claude Code folder")?;
    let proxy_files = files(env);
    let fs = StdFileSystem;
    let probe = |port: u16| healthy(port, PROBE);
    let report = Reconciler {
        enabled,
        port,
        shell_value: shell_base_url(),
        read_url: &|| Ok(read_base_url(&fs, &dir)?),
        write_url: &|url| Ok(write_base_url(&fs, &dir, url)?),
        known_urls: &|| proxy_files.known_base_urls(),
        remember_url: &|url| {
            let _ = proxy_files.remember_base_url(url);
        },
        healthy: &probe,
        start_proxy: &|| spawn_proxy(env, exe, &config.proxy.bind),
        wait_healthy: &|port| wait_until(settle, Duration::from_millis(50), &mut || probe(port)),
    }
    .run();
    Ok(report)
}

/// Writes the human-readable report: warnings to `err`, and the actions to `out` when `verbose`.
fn show(io: &mut Io, report: &Report, verbose: bool) -> Result<()> {
    write!(io.err(), "{}", report.warning_text())?;
    if verbose {
        let actions: String = report
            .actions
            .iter()
            .fold(String::new(), |mut text, action| {
                let _ = writeln!(text, "tokred: {action}");
                text
            });
        let nothing = if report.actions.is_empty() {
            format!("tokred: nothing to change ({})\n", report.state.name())
        } else {
            String::new()
        };
        write!(io.out(), "{actions}{nothing}")?;
    }
    Ok(())
}

/// The machine-readable report.
fn json_report(report: &Report) -> serde_json::Value {
    json!({
        "state": report.state.name(),
        "started": report.started,
        "actions": report.actions,
        "warnings": report.warnings,
    })
}

/// How long `initialize` waits for a proxy it started.
const SETTLE: Duration = Duration::from_secs(5);

/// `tokred initialize`: reconciles and reports; always `Ok`, whatever happens.
pub fn run(env: &Env, io: &mut Io, verbose: bool, json: bool) -> Result<()> {
    let outcome = std::env::current_exe()
        .context("could not locate the running executable")
        .and_then(|exe| {
            let config = housekeeping::load_config(env);
            reconcile(env, &config, &exe, config.proxy.enabled, SETTLE)
        });
    finish(io, verbose, json, outcome)
}

/// Prints the outcome of a reconcile; a failed one becomes a warning.
fn finish(io: &mut Io, verbose: bool, json: bool, outcome: Result<Report>) -> Result<()> {
    let report = outcome.unwrap_or_else(|error| {
        let mut report = Report::new(State::Skipped);
        report
            .warnings
            .push(format!("initialize skipped: {error:#}"));
        report
    });
    if json {
        let text = serde_json::to_string(&json_report(&report))?;
        writeln!(io.out(), "{text}")?;
        show(io, &report, false)?;
    } else {
        show(io, &report, verbose)?;
    }
    Ok(())
}

/// The `session-start` safety net: when the proxy is enabled and Claude Code is pointed at it
/// (by the environment of this session or by settings.json) but it does not answer within a
/// moment, starts it in the background without waiting. Returns whether it tried to.
pub fn ensure_for_session(env: &Env, config: &Config, exe: &Path) -> bool {
    let Ok(port) = port_of(&config.proxy.bind) else {
        return false;
    };
    let proxy_files = files(env);
    SessionCheck {
        enabled: config.proxy.enabled,
        port,
        shell_value: shell_base_url(),
        settings_value: &|| {
            settings_dir(env, config)
                .and_then(|dir| read_base_url(&StdFileSystem, &dir).ok().flatten())
        },
        known_urls: &|| proxy_files.known_base_urls(),
        healthy: &|port| healthy(port, HOOK_PROBE),
        start_proxy: &|| spawn_proxy(env, exe, &config.proxy.bind),
    }
    .run()
}

/// What [`ensure_for_session`] needs from the machine.
pub struct SessionCheck<'a> {
    /// `[proxy] enabled`.
    pub enabled: bool,
    /// The port of `[proxy] bind`.
    pub port: u16,
    /// `ANTHROPIC_BASE_URL` in the environment of the session.
    pub shell_value: Option<String>,
    /// `env.ANTHROPIC_BASE_URL` in settings.json.
    pub settings_value: &'a dyn Fn() -> Option<String>,
    /// The URLs tokred wrote before.
    pub known_urls: &'a dyn Fn() -> Vec<String>,
    /// Whether the proxy answers (with a short timeout).
    pub healthy: &'a dyn Fn(u16) -> bool,
    /// Starts the proxy in the background.
    pub start_proxy: &'a dyn Fn() -> Result<()>,
}

impl SessionCheck<'_> {
    /// Starts the proxy when it is enabled, in use by this session and not answering.
    pub fn run(&self) -> bool {
        if !self.enabled {
            return false;
        }
        let is_ours = |value: &str| is_tokred_url(value, self.port, self.known_urls);
        let in_use = self.shell_value.as_deref().is_some_and(is_ours)
            || (self.settings_value)().as_deref().is_some_and(is_ours);
        if !in_use || (self.healthy)(self.port) {
            return false;
        }
        (self.start_proxy)().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use anyhow::bail;

    use super::super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted_with_err;

    const OURS: &str = "http://127.0.0.1:1029";

    /// A fake machine: a settings value, a proxy that may be up, may start, may come up.
    #[allow(clippy::struct_excessive_bools)] // a fake with independent failure switches
    struct World {
        settings: RefCell<Option<String>>,
        known: RefCell<Vec<String>>,
        up: Cell<bool>,
        start_fails: bool,
        comes_up: bool,
        starts: Cell<u32>,
        writes: RefCell<Vec<Option<String>>>,
        read_fails: bool,
        write_fails: bool,
    }

    impl World {
        fn new(settings: Option<&str>, up: bool) -> Self {
            Self {
                settings: RefCell::new(settings.map(str::to_owned)),
                known: RefCell::new(Vec::new()),
                up: Cell::new(up),
                start_fails: false,
                comes_up: true,
                starts: Cell::new(0),
                writes: RefCell::new(Vec::new()),
                read_fails: false,
                write_fails: false,
            }
        }

        fn run(&self, enabled: bool, shell: Option<&str>) -> Report {
            let read = || -> Result<Option<String>> {
                if self.read_fails {
                    bail!("broken file");
                }
                Ok(self.settings.borrow().clone())
            };
            let write = |url: Option<&str>| -> Result<bool> {
                if self.write_fails {
                    bail!("read-only");
                }
                self.writes.borrow_mut().push(url.map(str::to_owned));
                *self.settings.borrow_mut() = url.map(str::to_owned);
                Ok(true)
            };
            let known = || self.known.borrow().clone();
            let remember = |url: &str| self.known.borrow_mut().push(url.to_owned());
            let healthy = |_port: u16| self.up.get();
            let start = || -> Result<()> {
                self.starts.set(self.starts.get() + 1);
                if self.start_fails {
                    bail!("permission denied");
                }
                self.up.set(self.comes_up);
                Ok(())
            };
            let wait = |_port: u16| self.up.get();
            Reconciler {
                enabled,
                port: 1029,
                shell_value: shell.map(str::to_owned),
                read_url: &read,
                write_url: &write,
                known_urls: &known,
                remember_url: &remember,
                healthy: &healthy,
                start_proxy: &start,
                wait_healthy: &wait,
            }
            .run()
        }

        fn url(&self) -> Option<String> {
            self.settings.borrow().clone()
        }
    }

    #[test]
    fn disabled_removes_only_tokred_urls() {
        let ours = World::new(Some(OURS), true);
        let report = ours.run(false, None);
        assert_eq!(report.state, State::Disabled);
        assert_eq!(ours.url(), None);
        assert!(report.actions[0].contains("removed"), "{report:?}");
        assert_eq!(ours.starts.get(), 0, "nothing is started");

        // An older URL tokred wrote (another port) is tokred's too.
        let older = World::new(Some("http://127.0.0.1:1029/"), false);
        older
            .known
            .borrow_mut()
            .push("http://127.0.0.1:1029".to_owned());
        older.run(false, None);
        assert_eq!(older.url(), None);

        // The user's own gateway, even a loopback one, stays.
        let foreign = World::new(Some("http://localhost:4000"), true);
        let report = foreign.run(false, None);
        assert_eq!(foreign.url().as_deref(), Some("http://localhost:4000"));
        assert!(report.actions[0].contains("left"), "{report:?}");
        assert!(foreign.writes.borrow().is_empty());

        let none = World::new(None, true);
        assert!(none.run(false, None).actions.is_empty());
    }

    #[test]
    fn disabled_warns_about_a_shell_variable_that_points_at_tokred() {
        let world = World::new(None, false);
        let report = world.run(false, Some(OURS));
        assert!(
            report.warnings[0].contains("exports ANTHROPIC_BASE_URL"),
            "{report:?}"
        );
        assert!(
            world
                .run(false, Some("https://gateway.example"))
                .warnings
                .is_empty()
        );
    }

    #[test]
    fn disabled_reports_a_failed_removal() {
        let mut world = World::new(Some(OURS), true);
        world.write_fails = true;
        let report = world.run(false, None);
        assert!(
            report.warnings[0].contains("could not remove"),
            "{report:?}"
        );
    }

    #[test]
    fn enabled_with_a_running_proxy_sets_the_url_once() {
        let world = World::new(None, true);
        let report = world.run(true, None);
        assert_eq!(report.state, State::Enabled);
        assert_eq!(world.url().as_deref(), Some(OURS));
        assert_eq!(*world.known.borrow(), [OURS]);
        assert!(!report.started && world.starts.get() == 0);

        let again = world.run(true, None);
        assert!(
            again.actions.is_empty() && again.warnings.is_empty(),
            "{again:?}"
        );
        assert_eq!(world.writes.borrow().len(), 1, "no rewrite when unchanged");
    }

    #[test]
    fn enabled_follows_a_changed_port() {
        let world = World::new(Some("http://127.0.0.1:1029"), true);
        world
            .known
            .borrow_mut()
            .push("http://127.0.0.1:1029".to_owned());
        let report = world.run(true, None);
        assert_eq!(report.state, State::Enabled);
        assert_eq!(world.url().as_deref(), Some(OURS));
    }

    #[test]
    fn enabled_starts_the_proxy_before_pointing_claude_at_it() {
        let world = World::new(None, false);
        let report = world.run(true, None);
        assert_eq!(report.state, State::Enabled);
        assert!(report.started);
        assert_eq!(world.starts.get(), 1);
        assert_eq!(world.url().as_deref(), Some(OURS));
        assert_eq!(report.actions.len(), 2, "{report:?}");
    }

    #[test]
    fn enabled_removes_the_url_again_when_the_proxy_cannot_be_started() {
        let mut cannot = World::new(Some(OURS), false);
        cannot.start_fails = true;
        let report = cannot.run(true, None);
        assert_eq!(report.state, State::Unavailable);
        assert_eq!(cannot.url(), None, "a URL for a dead proxy is removed");
        assert!(
            report.warnings[0].contains("permission denied"),
            "{report:?}"
        );

        let mut silent = World::new(None, false);
        silent.comes_up = false;
        let report = silent.run(true, None);
        assert_eq!(report.state, State::Unavailable);
        assert!(
            silent.writes.borrow().is_empty(),
            "never written, nothing to remove"
        );
        assert!(
            report.warnings[0].contains("did not answer on port 1029"),
            "{report:?}"
        );
        assert!(!report.started);

        let mut stuck = World::new(Some(OURS), false);
        stuck.start_fails = true;
        stuck.write_fails = true;
        let report = stuck.run(true, None);
        assert_eq!(report.warnings.len(), 2, "{report:?}");
        assert!(report.warnings[0].contains("could not remove"));
    }

    #[test]
    fn enabled_never_overwrites_another_gateway() {
        for (settings, shell) in [
            (Some("https://gateway.example"), None),
            (None, Some("https://gateway.example")),
            (Some(OURS), Some("https://gateway.example")),
        ] {
            let world = World::new(settings, false);
            let report = world.run(true, shell);
            assert_eq!(report.state, State::OtherGateway);
            assert!(
                report.warnings[0].contains("https://gateway.example"),
                "{report:?}"
            );
            assert_eq!(world.starts.get(), 0, "nothing is started");
            assert!(world.writes.borrow().is_empty());
            assert_eq!(world.url().as_deref(), settings);
        }
    }

    #[test]
    fn a_shell_variable_that_is_ours_is_not_another_gateway() {
        let world = World::new(None, true);
        let report = world.run(true, Some("http://127.0.0.1:1029/"));
        assert_eq!(report.state, State::Enabled);
        assert_eq!(world.url().as_deref(), Some(OURS));
    }

    #[test]
    fn enabled_reports_a_failed_write() {
        let mut world = World::new(None, true);
        world.write_fails = true;
        let report = world.run(true, None);
        assert!(report.warnings[0].contains("could not set"), "{report:?}");
    }

    #[test]
    fn an_unreadable_settings_file_changes_nothing() {
        let mut world = World::new(Some(OURS), false);
        world.read_fails = true;
        for enabled in [true, false] {
            let report = world.run(enabled, None);
            assert_eq!(report.state, State::Skipped);
            assert!(report.warnings[0].contains("broken file"), "{report:?}");
        }
        assert_eq!(world.starts.get(), 0);
        assert!(world.writes.borrow().is_empty());
    }

    #[test]
    fn the_state_names() {
        let names = [
            State::Disabled,
            State::Enabled,
            State::OtherGateway,
            State::Unavailable,
            State::Skipped,
        ]
        .map(State::name);
        assert_eq!(
            names,
            [
                "disabled",
                "enabled",
                "other-gateway",
                "unavailable",
                "skipped"
            ]
        );
    }

    /// A sandbox whose Claude Code folder is the one `claude` would use, with `[proxy]` set.
    fn sandbox(enabled: bool) -> (Sandbox, Config) {
        let mut sb = Sandbox::new();
        sb.env.claude_config_dir = Some(sb.claude.clone());
        let mut config = Config::default();
        config.proxy.enabled = enabled;
        config.proxy.bind = "127.0.0.1:1".to_owned();
        sb.write_settings(r#"{"model":"opus"}"#);
        (sb, config)
    }

    #[test]
    fn the_real_reconcile_edits_the_settings_file_and_remembers_the_url() {
        let (sb, mut config) = sandbox(true);
        // A proxy that answers: a fake listener on a free port.
        let port = fake_proxy();
        config.proxy.bind = format!("127.0.0.1:{port}");
        let exe = Path::new("/no/such/tokred");

        let report = reconcile(&sb.env, &config, exe, true, Duration::from_millis(100)).unwrap();
        assert_eq!(report.state, State::Enabled, "{report:?}");
        let url = format!("http://127.0.0.1:{port}");
        let text = std::fs::read_to_string(sb.settings()).unwrap();
        assert!(text.contains(&url), "{text}");
        assert_eq!(files(&sb.env).known_base_urls(), [url]);

        let report = reconcile(&sb.env, &config, exe, false, Duration::from_millis(100)).unwrap();
        assert_eq!(report.state, State::Disabled);
        let text = std::fs::read_to_string(sb.settings()).unwrap();
        assert_eq!(text.trim(), "{\n  \"model\": \"opus\"\n}");
    }

    #[test]
    fn the_real_reconcile_gives_up_when_the_proxy_cannot_start() {
        let (sb, config) = sandbox(true);
        let missing = Path::new("/no/such/tokred");
        let report = reconcile(&sb.env, &config, missing, true, Duration::from_millis(50)).unwrap();
        assert_eq!(report.state, State::Unavailable, "{report:?}");
        assert!(report.warnings[0].contains("could not start"), "{report:?}");
        let text = std::fs::read_to_string(sb.settings()).unwrap();
        assert!(!text.contains("ANTHROPIC_BASE_URL"));
    }

    #[test]
    fn the_real_reconcile_needs_a_folder_and_a_bind_address() {
        let (mut sb, mut config) = sandbox(true);
        config.proxy.bind = "nonsense".to_owned();
        let exe = Path::new("/x");
        assert!(reconcile(&sb.env, &config, exe, true, Duration::ZERO).is_err());
        config.proxy.bind = "127.0.0.1:1".to_owned();
        sb.env.claude_config_dir = None;
        assert!(reconcile(&sb.env, &config, exe, true, Duration::ZERO).is_err());
    }

    #[test]
    fn unconfigure_removes_only_a_url_that_is_tokreds() {
        let (sb, config) = sandbox(true);
        let url = |text: &str| format!(r#"{{"env":{{"ANTHROPIC_BASE_URL":"{text}"}}}}"#);
        sb.write_settings(&url("http://127.0.0.1:1"));
        let removed = remove_own_url(&sb.env, &config, &sb.claude).unwrap();
        assert_eq!(removed.as_deref(), Some("http://127.0.0.1:1"));
        assert_eq!(std::fs::read_to_string(sb.settings()).unwrap().trim(), "{}");

        // One an earlier port left behind is recognised from the record.
        files(&sb.env)
            .remember_base_url("http://127.0.0.1:1029")
            .unwrap();
        sb.write_settings(&url("http://127.0.0.1:1029"));
        assert!(
            remove_own_url(&sb.env, &config, &sb.claude)
                .unwrap()
                .is_some()
        );

        // The user's own gateway, no value at all, and no file are left alone.
        let foreign = url("https://gateway.example");
        sb.write_settings(&foreign);
        assert_eq!(remove_own_url(&sb.env, &config, &sb.claude).unwrap(), None);
        assert_eq!(std::fs::read_to_string(sb.settings()).unwrap(), foreign);
        sb.write_settings("{}");
        assert_eq!(remove_own_url(&sb.env, &config, &sb.claude).unwrap(), None);

        // A bind address that is not valid still recognises recorded URLs.
        let mut broken = config.clone();
        broken.proxy.bind = "nonsense".to_owned();
        sb.write_settings(&url("http://127.0.0.1:1029"));
        assert!(
            remove_own_url(&sb.env, &broken, &sb.claude)
                .unwrap()
                .is_some()
        );
        sb.write_settings("[[[");
        assert!(remove_own_url(&sb.env, &config, &sb.claude).is_err());
    }

    #[test]
    fn the_settings_folder_is_the_one_claude_would_use() {
        let (mut sb, mut config) = sandbox(true);
        assert_eq!(settings_dir(&sb.env, &config), Some(sb.claude.clone()));
        sb.env.claude_config_dir = None;
        config.claude.dir = Some("/saved".to_owned());
        sb.env.default_claude_dir = Some(PathBuf::from("/home/.claude"));
        assert_eq!(
            settings_dir(&sb.env, &config),
            Some(PathBuf::from("/saved"))
        );
        config.claude.dir = None;
        assert_eq!(
            settings_dir(&sb.env, &config),
            Some(PathBuf::from("/home/.claude"))
        );
    }

    /// Serves `{"ok":true}` to every request until the test process ends.
    fn fake_proxy() -> u16 {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0_u8; 1024];
                let _ = stream.read(&mut buffer);
                let body = r#"{"ok":true}"#;
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
            }
        });
        port
    }

    #[test]
    fn the_report_is_quiet_unless_verbose_and_warnings_go_to_stderr() {
        let mut report = Report::new(State::Enabled);
        report.actions.push("did a thing".to_owned());
        report.warnings.push("careful".to_owned());

        let (mut io, out, err) = scripted_with_err(false, "");
        show(&mut io, &report, false).unwrap();
        assert_eq!(out.text(), "");
        assert_eq!(err.text(), "tokred: careful\n");

        let (mut io, out, _) = scripted_with_err(false, "");
        show(&mut io, &report, true).unwrap();
        assert_eq!(out.text(), "tokred: did a thing\n");

        let quiet = Report::new(State::Disabled);
        let (mut io, out, err) = scripted_with_err(false, "");
        show(&mut io, &quiet, true).unwrap();
        assert_eq!(out.text(), "tokred: nothing to change (disabled)\n");
        assert_eq!(err.text(), "");
    }

    #[test]
    fn a_failed_reconcile_is_a_warning_and_never_an_error() {
        let (mut io, out, err) = scripted_with_err(false, "");
        finish(&mut io, false, true, Err(anyhow::anyhow!("no folder"))).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out.text()).unwrap();
        assert_eq!(value["state"], "skipped");
        assert_eq!(err.text(), "tokred: initialize skipped: no folder\n");
    }

    #[test]
    fn running_it_disabled_in_a_sandbox_changes_nothing_and_says_nothing() {
        let (sb, _) = sandbox(false);
        sb.write_settings(r#"{"env":{"ANTHROPIC_BASE_URL":"https://gateway.example"}}"#);
        let (mut io, out, err) = scripted_with_err(false, "");
        run(&sb.env, &mut io, false, false).unwrap();
        assert_eq!((out.text().as_str(), err.text().as_str()), ("", ""));
        let (mut io, out, _) = scripted_with_err(false, "");
        run(&sb.env, &mut io, true, false).unwrap();
        let text = out.text();
        assert!(
            text.contains("left ANTHROPIC_BASE_URL=https://gateway"),
            "{text}"
        );
    }

    #[test]
    fn the_json_report_has_the_state_and_what_happened() {
        let mut report = Report::new(State::OtherGateway);
        report.warnings.push("w".to_owned());
        let value = json_report(&report);
        assert_eq!(value["state"], "other-gateway");
        assert_eq!(value["started"], false);
        assert_eq!(value["warnings"][0], "w");
        assert_eq!(value["actions"], json!([]));
    }

    fn session(
        enabled: bool,
        shell: Option<&str>,
        settings: Option<&str>,
        up: bool,
        start_ok: bool,
    ) -> (bool, u32) {
        let starts = Cell::new(0);
        let settings_value = || settings.map(str::to_owned);
        let known = || vec!["http://127.0.0.1:1029".to_owned()];
        let healthy = |_port: u16| up;
        let start = || -> Result<()> {
            starts.set(starts.get() + 1);
            if start_ok { Ok(()) } else { bail!("no") }
        };
        let started = SessionCheck {
            enabled,
            port: 1029,
            shell_value: shell.map(str::to_owned),
            settings_value: &settings_value,
            known_urls: &known,
            healthy: &healthy,
            start_proxy: &start,
        }
        .run();
        (started, starts.get())
    }

    #[test]
    fn the_session_check_starts_a_dead_proxy_only_when_it_is_enabled_and_in_use() {
        assert_eq!(session(true, None, Some(OURS), false, true), (true, 1));
        assert_eq!(session(true, Some(OURS), None, false, true), (true, 1));
        assert_eq!(
            session(true, None, Some("http://127.0.0.1:1029"), false, true),
            (true, 1),
            "an older URL of tokred's counts"
        );
        assert_eq!(session(true, None, Some(OURS), false, false), (false, 1));
        assert_eq!(
            session(true, None, Some(OURS), true, true),
            (false, 0),
            "already up"
        );
        assert_eq!(
            session(false, None, Some(OURS), false, true),
            (false, 0),
            "disabled"
        );
        assert_eq!(
            session(true, None, None, false, true),
            (false, 0),
            "not in use"
        );
        assert_eq!(
            session(true, Some("https://gateway.example"), None, false, true),
            (false, 0),
            "another gateway"
        );
    }

    #[test]
    fn the_real_session_check_does_nothing_without_a_usable_setup() {
        let (sb, mut config) = sandbox(true);
        let exe = Path::new("/no/such/tokred");
        // Settings without the URL: the proxy is not in use.
        assert!(!ensure_for_session(&sb.env, &config, exe));
        // The URL is there, the proxy is down and cannot be started.
        let url = "http://127.0.0.1:1";
        sb.write_settings(&format!(r#"{{"env":{{"ANTHROPIC_BASE_URL":"{url}"}}}}"#));
        assert!(!ensure_for_session(&sb.env, &config, exe));
        // Another gateway is not in use by tokred, whatever the proxy is doing.
        sb.write_settings(r#"{"env":{"ANTHROPIC_BASE_URL":"https://gateway.example"}}"#);
        assert!(!ensure_for_session(&sb.env, &config, exe));
        config.proxy.bind = "garbage".to_owned();
        assert!(!ensure_for_session(&sb.env, &config, exe));
    }
}
