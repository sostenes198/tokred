//! `tokred uninstall`: everything tokred put on the machine except its own binary and the
//! shell setup, which the uninstall script removes once this process has exited.
//!
//! Every step is best effort: a failure is reported and the next step still runs, so a broken
//! proxy never keeps the hooks in Claude Code.

use std::path::PathBuf;

use anyhow::{Result, bail};
use tokred_install::StdFileSystem;

use super::proxy::{ProxyCtx, service_installed};
use super::{advice, select_claude_dir, unconfigure};
use crate::args::{ClaudeDirArg, CompressionCommand, ProxyCommand, ServiceCommand};
use crate::daemon::ServiceKind;
use crate::io::Io;

/// The exit code when the user answered "no" to the confirmation.
pub const DECLINED: i32 = 2;

/// What `uninstall` was asked for.
pub struct Request {
    /// `--claude-dir`.
    pub dir_flag: Option<PathBuf>,
    /// `--yes`.
    pub yes: bool,
    /// `--purge`.
    pub purge: bool,
}

pub fn run(io: &mut Io, ctx: &ProxyCtx<'_>, request: Request) -> Result<()> {
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

    let mut purge = request.purge;
    if !request.yes {
        if !io.interactive() {
            bail!("not running in a terminal; pass --yes to uninstall without asking");
        }
        let plan = "tokred uninstall will:\n  - stop the proxy and remove its user service, if installed\n  - turn compression off, if it is on\n  - remove tokred's hooks and agents from Claude Code\n";
        write!(io.out(), "{plan}")?;
        if !confirm(io, "Remove tokred from this machine? [y/N] ")? {
            writeln!(io.out(), "Nothing was changed.")?;
            // Exit code 2 tells the uninstall script not to delete the binary.
            return Err(super::ChildExit(DECLINED).into());
        }
        if !purge {
            let question =
                "Also delete tokred's data folder (database, config, logs, reports)? [y/N] ";
            purge = confirm(io, question)?;
        }
    }

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
        let done = unconfigure::apply(env, io, &dir, purge);
        step(io, "could not remove tokred from Claude Code", done);
    } else {
        let message = "No Claude Code folder found: nothing to unconfigure.";
        writeln!(io.out(), "{message}")?;
        if purge {
            let done = unconfigure::purge_data(env, io);
            step(io, "could not delete the data folder", done);
        }
    }

    let problems = if failures == 0 {
        String::new()
    } else {
        format!("\n{failures} step(s) above failed; fix them and run `tokred uninstall` again.\n")
    };
    let left = format!(
        "{problems}\nLeft for the uninstall script (or to do by hand): the tokred binary ({}), its folder in PATH and the `claude` wrapper in your shell profile.\n",
        ctx.exe.display()
    );
    write!(io.out(), "{left}")?;
    Ok(())
}

/// A yes/no question whose default (and end of input) is no.
fn confirm(io: &mut Io, question: &str) -> Result<bool> {
    let answer = io.ask(question)?.unwrap_or_default();
    Ok(matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes"))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use anyhow::ensure;
    use tokred_core::Config;

    use super::super::testing::{Sandbox, fake_health_server};
    use super::*;
    use crate::io::testing::scripted;

    /// Runs `uninstall` in the sandbox with a proxy that is not there and a recording
    /// `run_command` (no real `launchctl`/`systemctl`).
    fn uninstall(
        sb: &Sandbox,
        interactive: bool,
        input: &str,
        request: Request,
        bind: &str,
    ) -> (Result<()>, String, Vec<Vec<String>>) {
        uninstall_with(sb, interactive, input, request, bind, false)
    }

    /// [`uninstall`], where `task_exists` is what the Windows scheduler answers when asked for the
    /// proxy's logon task (the question is not recorded: only the removal commands are).
    fn uninstall_with(
        sb: &Sandbox,
        interactive: bool,
        input: &str,
        request: Request,
        bind: &str,
        task_exists: bool,
    ) -> (Result<()>, String, Vec<Vec<String>>) {
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
        let exe = std::path::Path::new("/opt/tokred/bin/tokred");
        let ctx = ProxyCtx {
            env: &sb.env,
            config: &config,
            exe,
            settle: Duration::from_millis(300),
            serve_foreground: &never,
            run_command: &record,
        };
        let (mut io, out) = scripted(interactive, input);
        let result = run(&mut io, &ctx, request);
        (result, out.text(), commands.into_inner())
    }

    fn request(sb: &Sandbox, yes: bool, purge: bool) -> Request {
        Request {
            dir_flag: Some(sb.claude.clone()),
            yes,
            purge,
        }
    }

    const NO_PROXY: &str = "127.0.0.1:1";

    fn configured() -> Sandbox {
        let sb = Sandbox::new();
        sb.configure();
        sb
    }

    #[test]
    fn everything_is_removed_and_the_data_folder_is_kept_by_default() {
        let sb = configured();
        let (result, out, commands) =
            uninstall(&sb, false, "", request(&sb, true, false), NO_PROXY);
        result.unwrap();
        assert!(out.contains("The tokred proxy is not running."), "{out}");
        assert!(out.contains("hooks removed from settings.json: 6"), "{out}");
        assert!(out.contains("Left for the uninstall script"), "{out}");
        assert!(out.contains("/opt/tokred/bin/tokred"), "{out}");
        assert!(!out.contains("  ! "), "{out}");
        assert!(commands.is_empty(), "no service was installed");
        assert!(!sb.agent("tokred-dev.md").exists());
        assert!(sb.env.data.root().exists());
    }

    #[test]
    fn purge_deletes_the_data_folder() {
        let sb = configured();
        let (result, out, _) = uninstall(&sb, false, "", request(&sb, true, true), NO_PROXY);
        result.unwrap();
        assert!(out.contains("data folder deleted"), "{out}");
        assert!(!sb.env.data.root().exists());
    }

    #[test]
    fn nothing_happens_without_a_terminal_or_yes() {
        let sb = configured();
        let (result, out, _) = uninstall(&sb, false, "y\n", request(&sb, false, false), NO_PROXY);
        assert!(result.unwrap_err().to_string().contains("--yes"));
        assert_eq!(out, "");
        assert!(sb.agent("tokred-dev.md").exists());
    }

    #[test]
    fn a_person_is_asked_to_confirm_and_may_say_no() {
        let sb = configured();
        for answer in ["n\n", "\n", ""] {
            let (result, out, _) =
                uninstall(&sb, true, answer, request(&sb, false, false), NO_PROXY);
            let error = result.unwrap_err();
            let declined = error.downcast_ref::<super::super::ChildExit>();
            assert_eq!(declined, Some(&super::super::ChildExit(DECLINED)));
            assert!(out.contains("Nothing was changed."), "{out}");
            assert!(sb.agent("tokred-dev.md").exists());
        }
    }

    #[test]
    fn a_person_is_asked_about_the_data_folder_unless_purge_was_given() {
        let sb = configured();
        let (result, out, _) = uninstall(&sb, true, "y\ny\n", request(&sb, false, false), NO_PROXY);
        result.unwrap();
        assert!(out.contains("Also delete tokred's data folder"), "{out}");
        assert!(!sb.env.data.root().exists());

        let sb = configured();
        let (result, out, _) = uninstall(&sb, true, "y\nn\n", request(&sb, false, false), NO_PROXY);
        result.unwrap();
        assert!(out.contains("Also delete tokred's data folder"), "{out}");
        assert!(sb.env.data.root().exists());

        let sb = configured();
        let (result, out, _) = uninstall(&sb, true, "yes\n", request(&sb, false, true), NO_PROXY);
        result.unwrap();
        assert!(!out.contains("Also delete"), "{out}");
        assert!(!sb.env.data.root().exists());
    }

    #[test]
    fn the_user_service_is_removed_only_when_it_is_installed() {
        let sb = configured();
        let kind = ServiceKind::for_os(std::env::consts::OS).unwrap();
        let exe = std::path::Path::new("/opt/tokred/bin/tokred");
        let home = sb.env.home_dir.clone().unwrap();
        let plan = crate::daemon::install_plan(kind, exe, &home);
        // The Windows logon task has no file: the scheduler answers instead.
        plan.file.iter().for_each(|(file, _)| {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "service").unwrap();
        });

        let (result, out, commands) =
            uninstall_with(&sb, false, "", request(&sb, true, false), NO_PROXY, true);
        result.unwrap();
        assert!(
            out.contains("Removed the tokred proxy user service."),
            "{out}"
        );
        assert!(
            !commands.is_empty(),
            "the service tool was asked to remove it"
        );
        plan.file
            .iter()
            .for_each(|(file, _)| assert!(!file.exists()));
    }

    #[test]
    fn a_failing_step_is_reported_and_the_rest_still_runs() {
        let sb = configured();
        // A listener that says it is a healthy proxy but never stops.
        let port = fake_health_server();
        let bind = format!("127.0.0.1:{port}");
        let (result, out, _) = uninstall(&sb, false, "", request(&sb, true, false), &bind);
        result.unwrap();
        assert!(out.contains("  ! could not stop the proxy"), "{out}");
        assert!(out.contains("1 step(s) above failed"), "{out}");
        assert!(
            !sb.agent("tokred-dev.md").exists(),
            "hooks and agents went anyway"
        );
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
        let (result, out, _) = uninstall(&sb, false, "", request(&sb, true, false), NO_PROXY);
        result.unwrap();
        assert!(out.contains("[compression] mode = \"observe\""), "{out}");
        let text = std::fs::read_to_string(&claude_json).unwrap();
        assert!(!text.contains("tokred"), "{text}");
    }

    #[test]
    fn without_a_claude_folder_only_the_rest_is_done() {
        let sb = Sandbox::new();
        sb.configure();
        let nowhere = Request {
            dir_flag: None,
            yes: true,
            purge: true,
        };
        // No flag and no saved folder that exists any more.
        std::fs::remove_dir_all(&sb.claude).unwrap();
        let (result, out, _) = uninstall(&sb, false, "", nowhere, NO_PROXY);
        result.unwrap();
        assert!(out.contains("No Claude Code folder found"), "{out}");
        assert!(out.contains("data folder deleted"), "{out}");

        let sb = Sandbox::new();
        let nowhere = Request {
            dir_flag: None,
            yes: true,
            purge: false,
        };
        let (result, out, _) = uninstall(&sb, false, "", nowhere, NO_PROXY);
        result.unwrap();
        assert!(out.contains("No Claude Code folder found"), "{out}");
    }

    #[test]
    fn a_bad_claude_dir_flag_is_an_error() {
        let sb = configured();
        let bad = Request {
            dir_flag: Some(sb.path("nope")),
            yes: true,
            purge: false,
        };
        let (result, _, _) = uninstall(&sb, false, "", bad, NO_PROXY);
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not a Claude Code folder")
        );
    }
}
