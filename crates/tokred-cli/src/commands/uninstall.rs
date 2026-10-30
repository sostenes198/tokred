//! `tokred uninstall`: removes tokred from the machine, completely and without asking.
//!
//! In order: the proxy (stopped, and its user service removed), the compression registration in
//! Claude Code, tokred's URL, hooks and agents in Claude Code, tokred's data folder, and last the
//! binary with its folder, the PATH entry and the `claude` wrapper (see [`super::cleanup`]).
//!
//! Every step is best effort: a failure is reported and the next step still runs, so a broken
//! proxy never keeps the hooks in Claude Code. But when any step failed the binary is kept, so
//! the leftovers still have a `tokred` to run `tokred uninstall` again.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use tokred_install::StdFileSystem;

use super::proxy::{ProxyCtx, service_installed};
use super::{advice, select_claude_dir, unconfigure};
use crate::args::{ClaudeDirArg, CompressionCommand, ProxyCommand, ServiceCommand};
use crate::daemon::ServiceKind;
use crate::env::Env;
use crate::io::Io;

/// What `uninstall` was asked for.
pub struct Request {
    /// `--claude-dir`.
    pub dir_flag: Option<PathBuf>,
}

/// The last step, once everything else went well: removes the binary, its folder, the PATH
/// entry and the `claude` wrapper.
pub type Finish<'a> = dyn Fn(&mut Io) -> Result<()> + 'a;

pub fn run(io: &mut Io, ctx: &ProxyCtx<'_>, request: Request, finish: &Finish<'_>) -> Result<()> {
    let env = ctx.env;
    // Without a flag a missing Claude Code folder is fine: there is nothing to unconfigure.
    let claude_dir = if request.dir_flag.is_some() {
        Some(select_claude_dir(
            &StdFileSystem,
            env,
            io,
            request.dir_flag,
            false,
        )?)
    } else {
        select_claude_dir(&StdFileSystem, env, io, None, false).ok()
    };

    let mut failures = 0_u32;
    let mut step = |io: &mut Io, what: &str, result: Result<()>| {
        if let Err(error) = result {
            failures += 1;
            let _ = writeln!(io.out(), "  ! {what}: {error:#}");
        }
    };

    let stopped = super::proxy::run(io, ctx, &ProxyCommand::Stop);
    step(io, "could not stop the proxy", stopped);
    if service_installed(ServiceKind::for_os(std::env::consts::OS), ctx) {
        let removed = super::proxy::run(
            io,
            ctx,
            &ProxyCommand::Service {
                command: ServiceCommand::Uninstall,
            },
        );
        step(io, "could not remove the user service", removed);
    }
    if let Some(dir) = claude_dir {
        if advice::compression_registered(env, &dir) {
            let off = CompressionCommand::Off {
                dir: ClaudeDirArg {
                    claude_dir: Some(dir.clone()),
                },
            };
            let done = advice::compression(env, io, &off, Some(dir.clone()), "");
            step(io, "could not turn compression off", done);
        }
        let done = remove_proxy_url(env, io, &dir);
        step(io, "could not remove the proxy URL from Claude Code", done);
        let done = unconfigure::apply(env, io, &dir, false);
        step(io, "could not remove tokred from Claude Code", done);
    } else {
        let message = "No Claude Code folder found: nothing to unconfigure.";
        writeln!(io.out(), "{message}")?;
    }
    let done = unconfigure::purge_data(env, io);
    step(io, "could not delete the data folder", done);

    if failures > 0 {
        bail!(
            "{failures} step(s) above failed, so the tokred binary was kept: fix them and run `tokred uninstall` again"
        );
    }
    finish(io)
}

/// Takes tokred's `ANTHROPIC_BASE_URL` out of Claude Code's settings (a gateway of the user's own
/// stays).
fn remove_proxy_url(env: &Env, io: &mut Io, claude_dir: &Path) -> Result<()> {
    let config = super::housekeeping::load_config(env);
    let removed = super::initialize::remove_own_url(env, &config, claude_dir)?;
    let line = removed
        .map(|url| format!("  ANTHROPIC_BASE_URL removed from settings.json: {url}\n"))
        .unwrap_or_default();
    write!(io.out(), "{line}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::time::Duration;

    use anyhow::{anyhow, ensure};
    use tokred_core::Config;

    use super::super::testing::{Sandbox, fake_health_server};
    use super::*;
    use crate::io::testing::scripted;

    /// What the runs below leave behind.
    struct Outcome {
        result: Result<()>,
        out: String,
        /// The system commands that were asked (not the scheduler query).
        commands: Vec<Vec<String>>,
        /// How many times the last step (binary, PATH, wrapper) was started.
        finished: usize,
    }

    fn uninstall(sb: &Sandbox, request: Request, bind: &str) -> Outcome {
        uninstall_with(sb, request, bind, false, &|_| Ok(()))
    }

    /// Runs `uninstall` in the sandbox with a proxy that is not there, a recording `run_command`
    /// (no real `launchctl`/`systemctl`), `task_exists` as the Windows scheduler's answer about
    /// the proxy's logon task, and `finish_with` as the last step.
    fn uninstall_with(
        sb: &Sandbox,
        request: Request,
        bind: &str,
        task_exists: bool,
        finish_with: &dyn Fn(&mut Io) -> Result<()>,
    ) -> Outcome {
        let mut config = Config::default();
        config.proxy.bind = bind.to_owned();
        let commands = RefCell::new(Vec::new());
        let record = |command: &[String]| -> Result<()> {
            let query = command == crate::daemon::task_query_command();
            if !query {
                commands.borrow_mut().push(command.to_vec());
            }
            ensure!(!query || task_exists, "no such task");
            Ok(())
        };
        let never = |_: &str| -> Result<()> { Ok(()) };
        let ctx = ProxyCtx {
            env: &sb.env,
            config: &config,
            exe: Path::new("/opt/tokred/bin/tokred"),
            settle: Duration::from_millis(300),
            serve_foreground: &never,
            run_command: &record,
        };
        let finished = Cell::new(0);
        let finish = |io: &mut Io| -> Result<()> {
            finished.set(finished.get() + 1);
            finish_with(io)
        };
        let (mut io, out) = scripted(false, "");
        let result = run(&mut io, &ctx, request, &finish);
        Outcome {
            result,
            out: out.text(),
            commands: commands.into_inner(),
            finished: finished.get(),
        }
    }

    fn request(sb: &Sandbox) -> Request {
        Request {
            dir_flag: Some(sb.claude.clone()),
        }
    }

    const NO_PROXY: &str = "127.0.0.1:1";

    fn configured() -> Sandbox {
        let sb = Sandbox::new();
        sb.configure();
        sb
    }

    #[test]
    fn everything_is_removed_without_asking_data_folder_included() {
        let sb = configured();
        tokred_install::base_url::write_base_url(
            &StdFileSystem,
            &sb.claude,
            Some("http://127.0.0.1:1029"),
        )
        .unwrap();
        let done = uninstall(&sb, request(&sb), NO_PROXY);
        done.result.unwrap();
        let out = done.out;
        assert!(out.contains("The tokred proxy is not running."), "{out}");
        assert!(out.contains("hooks removed from settings.json: 7"), "{out}");
        assert!(
            out.contains("ANTHROPIC_BASE_URL removed from settings.json: http://127.0.0.1:1029"),
            "{out}"
        );
        assert!(out.contains("data folder deleted"), "{out}");
        assert!(!out.contains("  ! "), "{out}");
        assert!(done.commands.is_empty(), "no service was installed");
        assert!(!sb.agent("tokred-dev.md").exists());
        assert!(!sb.env.data.root().exists(), "the data folder is gone too");
        assert_eq!(done.finished, 1, "the binary and the shell setup go last");
    }

    #[test]
    fn a_gateway_of_the_users_own_is_left_alone() {
        let sb = configured();
        tokred_install::base_url::write_base_url(
            &StdFileSystem,
            &sb.claude,
            Some("https://gateway.example"),
        )
        .unwrap();
        let done = uninstall(&sb, request(&sb), NO_PROXY);
        done.result.unwrap();
        assert!(
            !done.out.contains("ANTHROPIC_BASE_URL removed"),
            "{}",
            done.out
        );
        assert!(
            std::fs::read_to_string(sb.settings())
                .unwrap()
                .contains("gateway.example")
        );
    }

    #[test]
    fn the_user_service_is_removed_only_when_it_is_installed() {
        let sb = configured();
        let kind = ServiceKind::for_os(std::env::consts::OS).unwrap();
        let exe = Path::new("/opt/tokred/bin/tokred");
        let home = sb.env.home_dir.clone().unwrap();
        let plan = crate::daemon::install_plan(kind, exe, &home);
        // The Windows logon task has no file: the scheduler answers instead.
        plan.file.iter().for_each(|(file, _)| {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "service").unwrap();
        });

        let done = uninstall_with(&sb, request(&sb), NO_PROXY, true, &|_| Ok(()));
        done.result.unwrap();
        let out = done.out;
        assert!(
            out.contains("Removed the tokred proxy user service."),
            "{out}"
        );
        assert!(
            !done.commands.is_empty(),
            "the service tool was asked to remove it"
        );
        plan.file
            .iter()
            .for_each(|(file, _)| assert!(!file.exists()));
    }

    #[test]
    fn a_failing_step_is_reported_and_keeps_the_binary_but_the_rest_still_runs() {
        let sb = configured();
        // A listener that says it is a healthy proxy but never stops.
        let port = fake_health_server();
        let bind = format!("127.0.0.1:{port}");
        let done = uninstall(&sb, request(&sb), &bind);
        let error = done.result.unwrap_err().to_string();
        assert!(error.contains("1 step(s) above failed"), "{error}");
        assert!(error.contains("binary was kept"), "{error}");
        assert!(
            done.out.contains("  ! could not stop the proxy"),
            "{}",
            done.out
        );
        assert_eq!(done.finished, 0, "the binary stays");
        assert!(
            !sb.agent("tokred-dev.md").exists(),
            "hooks and agents went anyway"
        );
    }

    #[test]
    fn a_failing_last_step_is_the_result() {
        let sb = configured();
        let done = uninstall_with(&sb, request(&sb), NO_PROXY, false, &|_| {
            Err(anyhow!("no sh"))
        });
        assert!(done.result.unwrap_err().to_string().contains("no sh"));
        assert_eq!(done.finished, 1);
    }

    #[test]
    fn compression_is_turned_off_only_when_it_was_registered() {
        let sb = configured();
        let claude_json = sb.env.home_dir.clone().unwrap().join(".claude.json");
        std::fs::write(
            &claude_json,
            r#"{"mcpServers":{"tokred":{"command":"/x/tokred","args":["mcp"]}}}"#,
        )
        .unwrap();
        let done = uninstall(&sb, request(&sb), NO_PROXY);
        done.result.unwrap();
        assert!(
            done.out.contains("[compression] mode = \"observe\""),
            "{}",
            done.out
        );
        let text = std::fs::read_to_string(&claude_json).unwrap();
        assert!(!text.contains("tokred"), "{text}");
    }

    #[test]
    fn without_a_claude_folder_the_data_folder_and_the_binary_still_go() {
        let sb = Sandbox::new();
        sb.configure();
        // No flag and no saved folder that exists any more.
        std::fs::remove_dir_all(&sb.claude).unwrap();
        let done = uninstall(&sb, Request { dir_flag: None }, NO_PROXY);
        done.result.unwrap();
        assert!(
            done.out.contains("No Claude Code folder found"),
            "{}",
            done.out
        );
        assert!(done.out.contains("data folder deleted"), "{}", done.out);
        assert!(!sb.env.data.root().exists());
        assert_eq!(done.finished, 1);

        // Nothing installed at all: still fine.
        let bare = Sandbox::new();
        let done = uninstall(&bare, Request { dir_flag: None }, NO_PROXY);
        done.result.unwrap();
        assert_eq!(done.finished, 1);
    }

    #[test]
    fn a_bad_claude_dir_flag_is_an_error() {
        let sb = configured();
        let bad = Request {
            dir_flag: Some(sb.path("nope")),
        };
        let done = uninstall(&sb, bad, NO_PROXY);
        assert!(
            done.result
                .unwrap_err()
                .to_string()
                .contains("not a Claude Code folder")
        );
        assert_eq!(done.finished, 0);
    }
}
