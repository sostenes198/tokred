//! `tokred proxy start|stop|status|service`.
//!
//! The background proxy is this same binary running `tokred proxy start --foreground`. It writes
//! `<data>/proxy/pid`, answers `GET /__tokred/health`, and accepts `GET /__tokred/status` and
//! `POST /__tokred/shutdown` only with the secret from `<data>/proxy/secret`.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokred_core::Config;

use crate::args::{ProxyCommand, ServiceCommand};
use crate::daemon::{
    ProxyFiles, SECRET_HEADER, ServiceKind, base_url, healthy, install_plan, port_of,
    spawn_detached, uninstall_plan, wait_until,
};
use crate::env::Env;
use crate::io::Io;

/// Timeout of one health or admin request.
const ADMIN_TIMEOUT: Duration = Duration::from_secs(2);
/// How long `start` and `stop` wait for the proxy to come up or go away.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);

/// The proxy's files for this data folder.
#[must_use]
pub fn files(env: &Env) -> ProxyFiles {
    ProxyFiles::new(env.data.root().join("proxy"))
}

/// What the proxy commands need besides the terminal.
pub struct ProxyCtx<'a> {
    /// Locations.
    pub env: &'a Env,
    /// `tokred.toml`.
    pub config: &'a Config,
    /// The running tokred executable (spawned for the background proxy).
    pub exe: &'a Path,
    /// Runs the proxy in this process until it stops (the foreground mode).
    pub serve_foreground: &'a dyn Fn(&str) -> Result<()>,
    /// Runs a system command (`systemctl`, `launchctl`, `schtasks`) for the user service.
    pub run_command: &'a dyn Fn(&[String]) -> Result<()>,
}

pub fn run(io: &mut Io, ctx: &ProxyCtx<'_>, command: &ProxyCommand) -> Result<()> {
    match command {
        ProxyCommand::Start { foreground, bind } => {
            let bind = bind
                .clone()
                .unwrap_or_else(|| ctx.config.proxy.bind.clone());
            if *foreground {
                (ctx.serve_foreground)(&bind)
            } else {
                start_background(io, ctx, &bind)
            }
        }
        ProxyCommand::Stop => stop(io, ctx),
        ProxyCommand::Status { json } => status(io, ctx, *json),
        ProxyCommand::Service { command } => service(io, ctx, command),
    }
}

fn start_background(io: &mut Io, ctx: &ProxyCtx<'_>, bind: &str) -> Result<()> {
    let port = port_of(bind)?;
    if healthy(port, ADMIN_TIMEOUT) {
        writeln!(
            io.out(),
            "The tokred proxy is already running on {}.",
            base_url(port)
        )?;
        return Ok(());
    }
    // Create the secret before the child needs it.
    files(ctx.env).secret()?;
    spawn_detached(ctx.exe, &["proxy", "start", "--foreground", "--bind", bind])?;
    if !wait_until(SETTLE_TIMEOUT, Duration::from_millis(100), &mut || {
        healthy(port, ADMIN_TIMEOUT)
    }) {
        bail!(
            "the proxy did not answer on {} within {}s; see the logs in {}",
            base_url(port),
            SETTLE_TIMEOUT.as_secs(),
            ctx.env.data.logs_dir().display()
        );
    }
    writeln!(
        io.out(),
        "The tokred proxy is running on {}.",
        base_url(port)
    )?;
    writeln!(io.out(), "Start Claude Code through it with: tokred claude")?;
    Ok(())
}

fn admin(port: u16, secret: &str, method: &str, path: &str) -> Result<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(ADMIN_TIMEOUT))
        .build()
        .into();
    let url = format!("{}{path}", base_url(port));
    let mut response = if method == "POST" {
        agent.post(&url).header(SECRET_HEADER, secret).send_empty()
    } else {
        agent.get(&url).header(SECRET_HEADER, secret).call()
    }
    .with_context(|| format!("the proxy refused {method} {path}"))?;
    Ok(response.body_mut().read_to_string()?)
}

fn stop(io: &mut Io, ctx: &ProxyCtx<'_>) -> Result<()> {
    let files = files(ctx.env);
    let port = files
        .read_pid()
        .map_or_else(|| port_of(&ctx.config.proxy.bind), |(_, port)| Ok(port))?;
    if !healthy(port, ADMIN_TIMEOUT) {
        files.clear_pid();
        writeln!(io.out(), "The tokred proxy is not running.")?;
        return Ok(());
    }
    admin(port, &files.secret()?, "POST", "/__tokred/shutdown")?;
    if !wait_until(SETTLE_TIMEOUT, Duration::from_millis(100), &mut || {
        !healthy(port, ADMIN_TIMEOUT)
    }) {
        bail!("the proxy on port {port} did not stop");
    }
    files.clear_pid();
    writeln!(io.out(), "The tokred proxy stopped.")?;
    Ok(())
}

fn service_installed(kind: Option<ServiceKind>, ctx: &ProxyCtx<'_>) -> bool {
    match (kind, &ctx.env.home_dir) {
        (Some(kind), Some(home)) => install_plan(kind, ctx.exe, home)
            .file
            .is_some_and(|(path, _)| path.exists()),
        _ => false,
    }
}

fn status(io: &mut Io, ctx: &ProxyCtx<'_>, json: bool) -> Result<()> {
    let files = files(ctx.env);
    let port = files
        .read_pid()
        .map_or_else(|| port_of(&ctx.config.proxy.bind), |(_, port)| Ok(port))?;
    let running = healthy(port, ADMIN_TIMEOUT);
    let details: Value = if running {
        files
            .secret()
            .ok()
            .and_then(|secret| admin(port, &secret, "GET", "/__tokred/status").ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let base = std::env::var("ANTHROPIC_BASE_URL").ok();
    let points_here = base
        .as_deref()
        .is_some_and(|url| url.trim_end_matches('/') == base_url(port));
    let kind = ServiceKind::for_os(std::env::consts::OS);
    let value = json!({
        "running": running,
        "url": base_url(port),
        "pid": files.read_pid().map(|(pid, _)| pid),
        "service_installed": service_installed(kind, ctx),
        "anthropic_base_url": base,
        "anthropic_base_url_points_here": points_here,
        "routing_mode": ctx.config.routing.mode,
        "compression_mode": ctx.config.compression.mode,
        "details": details,
    });
    if json {
        writeln!(io.out(), "{}", serde_json::to_string_pretty(&value)?)?;
        return Ok(());
    }
    let state = if running { "running" } else { "not running" };
    writeln!(io.out(), "tokred proxy: {state} ({})", base_url(port))?;
    writeln!(
        io.out(),
        "  user service:  {}",
        if value["service_installed"] == true {
            "installed"
        } else {
            "not installed"
        }
    )?;
    writeln!(
        io.out(),
        "  this terminal: {}",
        if points_here {
            "ANTHROPIC_BASE_URL points to the proxy"
        } else {
            "ANTHROPIC_BASE_URL does not point to the proxy (use `tokred claude`)"
        }
    )?;
    writeln!(
        io.out(),
        "  routing:       {}   compression: {}",
        value["routing_mode"].as_str().unwrap_or("?"),
        value["compression_mode"].as_str().unwrap_or("?")
    )?;
    Ok(())
}

fn service(io: &mut Io, ctx: &ProxyCtx<'_>, command: &ServiceCommand) -> Result<()> {
    let kind = ServiceKind::for_os(std::env::consts::OS)
        .context("user services are not supported on this operating system")?;
    let home = ctx
        .env
        .home_dir
        .as_deref()
        .context("could not determine the home folder")?;
    match command {
        ServiceCommand::Install => {
            let plan = install_plan(kind, ctx.exe, home);
            if let Some((path, content)) = &plan.file {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("could not create {}", parent.display()))?;
                }
                std::fs::write(path, content)
                    .with_context(|| format!("could not write {}", path.display()))?;
            }
            for command in &plan.commands {
                (ctx.run_command)(command)?;
            }
            writeln!(
                io.out(),
                "Installed the tokred proxy as a user service; it starts at login."
            )?;
        }
        ServiceCommand::Uninstall => {
            let plan = uninstall_plan(kind, home);
            for command in &plan.commands {
                // Already stopped or never installed: keep going and remove the file.
                let _ = (ctx.run_command)(command);
            }
            if let Some((path, _)) = &plan.file {
                let _ = std::fs::remove_file(path);
            }
            writeln!(io.out(), "Removed the tokred proxy user service.")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::super::testing::Sandbox;
    use super::*;
    use crate::io::testing::scripted;

    /// A fake proxy: health, status and shutdown, checking the secret.
    fn fake_proxy(secret: String) -> (u16, Arc<AtomicBool>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stopped);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut buffer = [0_u8; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).to_lowercase();
                let authorized = request.contains(&format!("x-tokred-secret: {secret}"));
                let (code, body) = if flag.load(Ordering::SeqCst) {
                    break;
                } else if request.starts_with("get /__tokred/health") {
                    (200, r#"{"ok":true}"#.to_owned())
                } else if request.starts_with("get /__tokred/status") && authorized {
                    (200, r#"{"requests":3}"#.to_owned())
                } else if request.starts_with("post /__tokred/shutdown") && authorized {
                    flag.store(true, Ordering::SeqCst);
                    (200, "{}".to_owned())
                } else {
                    (403, "{}".to_owned())
                };
                let response = format!(
                    "HTTP/1.1 {code} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (port, stopped)
    }

    fn ctx<'a>(
        sb: &'a Sandbox,
        config: &'a Config,
        serve: &'a dyn Fn(&str) -> Result<()>,
    ) -> ProxyCtx<'a> {
        ProxyCtx {
            env: &sb.env,
            config,
            exe: Path::new("/no/such/tokred"),
            serve_foreground: serve,
            run_command: &refuse_commands,
        }
    }

    /// Tests never run real system commands.
    fn refuse_commands(command: &[String]) -> Result<()> {
        bail!("refused: {}", command.join(" "))
    }

    fn config_on(port: u16) -> Config {
        let mut config = Config::default();
        config.proxy.bind = format!("127.0.0.1:{port}");
        config
    }

    fn no_serve(_: &str) -> Result<()> {
        bail!("not in this test")
    }

    #[test]
    fn foreground_start_runs_the_proxy_in_this_process() {
        let sb = Sandbox::new();
        let config = Config::default();
        let served = std::cell::RefCell::new(String::new());
        let serve = |bind: &str| -> Result<()> {
            served.borrow_mut().push_str(bind);
            Ok(())
        };
        let (mut io, _) = scripted(false, "");
        let command = ProxyCommand::Start {
            foreground: true,
            bind: Some("127.0.0.1:9999".to_owned()),
        };
        run(&mut io, &ctx(&sb, &config, &serve), &command).unwrap();
        assert_eq!(*served.borrow(), "127.0.0.1:9999");
    }

    #[test]
    fn background_start_reports_an_already_running_proxy_or_a_failure() {
        let sb = Sandbox::new();
        let secret = files(&sb.env).secret().unwrap();
        let (port, _) = fake_proxy(secret);
        let config = config_on(port);
        let (mut io, out) = scripted(false, "");
        let command = ProxyCommand::Start {
            foreground: false,
            bind: None,
        };
        run(&mut io, &ctx(&sb, &config, &no_serve), &command).unwrap();
        assert!(out.text().contains("already running"), "{}", out.text());

        // Nothing listens and the executable does not exist: a clear error.
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let config = config_on(closed);
        let (mut io, _) = scripted(false, "");
        let err = run(&mut io, &ctx(&sb, &config, &no_serve), &command).unwrap_err();
        assert!(format!("{err:#}").contains("could not start"), "{err:#}");
    }

    #[test]
    fn stop_and_status_talk_to_the_admin_endpoints_with_the_secret() {
        let sb = Sandbox::new();
        let secret = files(&sb.env).secret().unwrap();
        let (port, stopped) = fake_proxy(secret);
        files(&sb.env).write_pid(1234, port).unwrap();
        let config = config_on(port);

        let (mut io, out) = scripted(false, "");
        run(
            &mut io,
            &ctx(&sb, &config, &no_serve),
            &ProxyCommand::Status { json: true },
        )
        .unwrap();
        let value: Value = serde_json::from_str(&out.text()).unwrap();
        assert_eq!(value["running"], true);
        assert_eq!(value["pid"], 1234);
        assert_eq!(value["details"]["requests"], 3, "status needs the secret");
        assert_eq!(value["routing_mode"], "observe");

        let (mut io, out) = scripted(false, "");
        run(
            &mut io,
            &ctx(&sb, &config, &no_serve),
            &ProxyCommand::Status { json: false },
        )
        .unwrap();
        assert!(
            out.text().contains("tokred proxy: running"),
            "{}",
            out.text()
        );
        assert!(out.text().contains("routing:       observe"));

        let (mut io, out) = scripted(false, "");
        run(&mut io, &ctx(&sb, &config, &no_serve), &ProxyCommand::Stop).unwrap();
        assert!(out.text().contains("stopped"), "{}", out.text());
        assert!(stopped.load(Ordering::SeqCst));
        assert_eq!(files(&sb.env).read_pid(), None);
    }

    #[test]
    fn stop_and_status_when_nothing_runs() {
        let sb = Sandbox::new();
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let config = config_on(closed);
        let (mut io, out) = scripted(false, "");
        run(&mut io, &ctx(&sb, &config, &no_serve), &ProxyCommand::Stop).unwrap();
        assert!(out.text().contains("not running"));
        let (mut io, out) = scripted(false, "");
        run(
            &mut io,
            &ctx(&sb, &config, &no_serve),
            &ProxyCommand::Status { json: false },
        )
        .unwrap();
        assert!(
            out.text().contains("tokred proxy: not running"),
            "{}",
            out.text()
        );
        assert!(out.text().contains("use `tokred claude`"));
    }

    #[test]
    fn the_service_is_installed_and_removed_through_the_command_runner() {
        let Some(kind) = ServiceKind::for_os(std::env::consts::OS) else {
            return;
        };
        let sb = Sandbox::new();
        let config = Config::default();
        let ran = std::cell::RefCell::new(Vec::new());
        let record = |command: &[String]| -> Result<()> {
            ran.borrow_mut().push(command.join(" "));
            Ok(())
        };
        let ctx = ProxyCtx {
            run_command: &record,
            ..ctx(&sb, &config, &no_serve)
        };
        let home = sb.env.home_dir.clone().unwrap();

        let (mut io, out) = scripted(false, "");
        let install = ProxyCommand::Service {
            command: ServiceCommand::Install,
        };
        run(&mut io, &ctx, &install).unwrap();
        assert!(
            out.text()
                .contains("Installed the tokred proxy as a user service")
        );
        let plan = install_plan(kind, ctx.exe, &home);
        if let Some((path, content)) = &plan.file {
            assert_eq!(std::fs::read_to_string(path).unwrap(), *content);
        }
        assert_eq!(ran.borrow().len(), plan.commands.len());
        assert_eq!(service_installed(Some(kind), &ctx), plan.file.is_some());

        let (mut io, out) = scripted(false, "");
        let uninstall = ProxyCommand::Service {
            command: ServiceCommand::Uninstall,
        };
        run(&mut io, &ctx, &uninstall).unwrap();
        assert!(out.text().contains("Removed the tokred proxy user service"));
        if let Some((path, _)) = &plan.file {
            assert!(!path.exists());
        }

        // A failing system command stops the install; uninstall carries on.
        let failing = ctx_with_refusal(&sb, &config);
        let (mut io, _) = scripted(false, "");
        assert!(
            run(&mut io, &failing, &install)
                .unwrap_err()
                .to_string()
                .contains("refused")
        );
        let (mut io, _) = scripted(false, "");
        run(&mut io, &failing, &uninstall).unwrap();
        assert!(!service_installed(None, &failing));
    }

    fn ctx_with_refusal<'a>(sb: &'a Sandbox, config: &'a Config) -> ProxyCtx<'a> {
        ctx(sb, config, &no_serve)
    }

    #[test]
    fn a_bad_bind_address_is_an_error() {
        let sb = Sandbox::new();
        let mut config = Config::default();
        config.proxy.bind = "nowhere".to_owned();
        let (mut io, _) = scripted(false, "");
        let command = ProxyCommand::Start {
            foreground: false,
            bind: None,
        };
        assert!(run(&mut io, &ctx(&sb, &config, &no_serve), &command).is_err());
    }
}
