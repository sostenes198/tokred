//! `tokred claude [args...]`: runs Claude Code through the local proxy, for this run only.
//!
//! Nothing is written to Claude Code's settings. If the proxy cannot be started, or the user
//! already points Claude Code at another gateway, `claude` runs unchanged and a note goes to
//! stderr: tokred never stops Claude Code from working.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::daemon::base_url;
use crate::io::Io;

/// How long to wait for a freshly started proxy.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs `claude` with an optional extra environment variable and its arguments.
pub type RunClaude<'a> = dyn Fn(Option<(&str, &str)>, &[OsString]) -> Result<i32> + 'a;

/// What the launcher needs from the outside world.
pub struct Launcher<'a> {
    /// `[proxy] enabled`: when off, `claude` runs directly.
    pub enabled: bool,
    /// The port the proxy listens on.
    pub port: u16,
    /// The `ANTHROPIC_BASE_URL` already in the environment, if any.
    pub existing_base_url: Option<String>,
    /// Whether the proxy answers on `port`.
    pub healthy: &'a dyn Fn(u16) -> bool,
    /// Starts the proxy in the background.
    pub start_proxy: &'a dyn Fn() -> Result<()>,
    /// Waits up to the given time for the proxy to become healthy.
    pub wait_healthy: &'a dyn Fn(u16, Duration) -> bool,
    /// Runs `claude` with the given extra environment and arguments; returns its exit code.
    pub run_claude: &'a RunClaude<'a>,
}

/// Why `claude` runs without the proxy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bypass {
    /// `[proxy] enabled = false`.
    Disabled,
    /// Another gateway is already configured.
    OtherGateway(String),
    /// The proxy did not start.
    NotStarted(String),
}

/// Decides whether to use the proxy, starting it if needed. `Ok(None)` means: use it.
pub fn prepare(launcher: &Launcher<'_>) -> Option<Bypass> {
    if !launcher.enabled {
        return Some(Bypass::Disabled);
    }
    let ours = base_url(launcher.port);
    if let Some(existing) = &launcher.existing_base_url
        && existing.trim_end_matches('/') != ours
    {
        return Some(Bypass::OtherGateway(existing.clone()));
    }
    if (launcher.healthy)(launcher.port) {
        return None;
    }
    if let Err(err) = (launcher.start_proxy)() {
        return Some(Bypass::NotStarted(format!("{err:#}")));
    }
    if (launcher.wait_healthy)(launcher.port, START_TIMEOUT) {
        None
    } else {
        Some(Bypass::NotStarted(format!(
            "it did not answer on port {} within {}s",
            launcher.port,
            START_TIMEOUT.as_secs()
        )))
    }
}

/// Runs Claude Code, through the proxy when possible. Returns `claude`'s exit code.
pub fn run(io: &mut Io, launcher: &Launcher<'_>, args: &[OsString]) -> Result<i32> {
    let url = base_url(launcher.port);
    match prepare(launcher) {
        None => (launcher.run_claude)(Some(("ANTHROPIC_BASE_URL", &url)), args),
        Some(bypass) => {
            let reason = match bypass {
                Bypass::Disabled => "the tokred proxy is disabled ([proxy] enabled = false); running claude without it (turn it on with `tokred proxy enable`)".to_owned(),
                Bypass::OtherGateway(existing) => format!(
                    "ANTHROPIC_BASE_URL is already set to {existing}; running claude without the tokred proxy"
                ),
                Bypass::NotStarted(why) => {
                    format!(
                        "the tokred proxy could not be started ({why}); running claude without it"
                    )
                }
            };
            writeln!(io.err(), "tokred: {reason}")?;
            (launcher.run_claude)(None, args)
        }
    }
}

/// Finds `claude` on `PATH`.
pub fn find_claude() -> Result<PathBuf> {
    which::which("claude").context("could not find `claude` on PATH; is Claude Code installed?")
}

/// Runs `program args...` with inherited standard streams and an optional extra variable.
pub fn spawn_inherit(program: &Path, env: Option<(&str, &str)>, args: &[OsString]) -> Result<i32> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    if let Some((name, value)) = env {
        command.env(name, value);
    }
    let status = command
        .status()
        .with_context(|| format!("could not run {}", program.display()))?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use anyhow::bail;

    use super::*;
    use crate::io::testing::scripted_with_err;

    struct World {
        healthy_now: Cell<bool>,
        starts: Cell<u32>,
        start_fails: bool,
        comes_up: bool,
        ran: RefCell<Vec<(Option<String>, Vec<OsString>)>>,
    }

    impl World {
        fn new(healthy: bool, start_fails: bool, comes_up: bool) -> Self {
            Self {
                healthy_now: Cell::new(healthy),
                starts: Cell::new(0),
                start_fails,
                comes_up,
                ran: RefCell::new(Vec::new()),
            }
        }
    }

    fn go(world: &World, existing: Option<&str>) -> (i32, String) {
        go_with(world, existing, true)
    }

    fn go_with(world: &World, existing: Option<&str>, enabled: bool) -> (i32, String) {
        let healthy = |_port: u16| world.healthy_now.get();
        let start = || -> Result<()> {
            world.starts.set(world.starts.get() + 1);
            if world.start_fails {
                bail!("permission denied");
            }
            Ok(())
        };
        let wait = |_port: u16, _timeout: Duration| world.comes_up;
        let run_claude = |env: Option<(&str, &str)>, args: &[OsString]| -> Result<i32> {
            world
                .ran
                .borrow_mut()
                .push((env.map(|(k, v)| format!("{k}={v}")), args.to_vec()));
            Ok(7)
        };
        let launcher = Launcher {
            enabled,
            port: 1029,
            existing_base_url: existing.map(str::to_owned),
            healthy: &healthy,
            start_proxy: &start,
            wait_healthy: &wait,
            run_claude: &run_claude,
        };
        let (mut io, _out, err) = scripted_with_err(false, "");
        let code = run(
            &mut io,
            &launcher,
            &[OsString::from("-p"), OsString::from("hi")],
        )
        .unwrap();
        (code, err.text())
    }

    #[test]
    fn a_healthy_proxy_is_used_for_this_run_only() {
        let world = World::new(true, false, true);
        let (code, err) = go(&world, None);
        assert_eq!(code, 7, "claude's exit code is returned");
        assert_eq!(err, "");
        assert_eq!(world.starts.get(), 0);
        let ran = world.ran.borrow();
        assert_eq!(
            ran[0].0.as_deref(),
            Some("ANTHROPIC_BASE_URL=http://127.0.0.1:1029")
        );
        assert_eq!(ran[0].1, [OsString::from("-p"), OsString::from("hi")]);
    }

    #[test]
    fn a_stopped_proxy_is_started_first() {
        let world = World::new(false, false, true);
        let (_, err) = go(&world, None);
        assert_eq!(world.starts.get(), 1);
        assert_eq!(err, "");
        assert!(world.ran.borrow()[0].0.is_some());
    }

    #[test]
    fn our_own_base_url_counts_as_no_other_gateway() {
        let world = World::new(true, false, true);
        go(&world, Some("http://127.0.0.1:1029/"));
        assert!(world.ran.borrow()[0].0.is_some());
    }

    #[test]
    fn a_disabled_proxy_means_claude_runs_directly_and_says_so() {
        let world = World::new(true, false, true);
        let (code, err) = go_with(&world, None, false);
        assert_eq!(code, 7);
        assert!(
            err.contains("proxy is disabled ([proxy] enabled = false)"),
            "{err}"
        );
        assert_eq!(err.matches('\n').count(), 1, "said once");
        assert_eq!(world.starts.get(), 0, "nothing is started");
        assert!(world.ran.borrow()[0].0.is_none(), "claude runs unchanged");
    }

    #[test]
    fn another_gateway_is_respected() {
        let world = World::new(true, false, true);
        let (_, err) = go(&world, Some("https://gateway.example"));
        assert!(
            err.contains("already set to https://gateway.example"),
            "{err}"
        );
        assert!(world.ran.borrow()[0].0.is_none(), "claude runs unchanged");
    }

    #[test]
    fn claude_still_runs_when_the_proxy_cannot_start() {
        let failing = World::new(false, true, false);
        let (code, err) = go(&failing, None);
        assert_eq!(code, 7);
        assert!(
            err.contains("could not be started (permission denied)"),
            "{err}"
        );
        assert!(failing.ran.borrow()[0].0.is_none());

        let silent = World::new(false, false, false);
        let (_, err) = go(&silent, None);
        assert!(
            err.contains("did not answer on port 1029 within 5s"),
            "{err}"
        );
        assert!(silent.ran.borrow()[0].0.is_none());
    }

    #[test]
    fn programs_run_with_inherited_streams_and_report_their_code() {
        let me = std::env::current_exe().unwrap();
        let code = spawn_inherit(
            &me,
            Some(("TOKRED_TEST_VAR", "1")),
            &[OsString::from("--list")],
        )
        .unwrap();
        assert_eq!(code, 0);
        assert_ne!(
            spawn_inherit(&me, None, &[OsString::from("--definitely-not-a-flag")]).unwrap(),
            0
        );
        assert!(spawn_inherit(Path::new("/no/such/claude"), None, &[]).is_err());
        // `claude` may or may not be installed where the tests run; both outcomes are valid.
        let _ = find_claude();
    }
}
