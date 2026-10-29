//! `tokred proxy start|stop|enable|disable|status|service`.
//!
//! The background proxy is this same binary running `tokred proxy start --foreground`. It writes
//! `<data>/proxy/pid`, answers `GET /__tokred/health`, and accepts `GET /__tokred/status` and
//! `POST /__tokred/shutdown` only with the secret from `<data>/proxy/secret`.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokred_core::config::ProxyConfig;
use tokred_core::proxy::ProxyRequestRecord;
use tokred_core::{Clock, Config, Timestamp};

use super::advice::set_key;
use super::port::{self, PortState};
use super::{SystemClock, existing_store, initialize};
use crate::args::{ProxyCommand, ServiceCommand};
use crate::daemon::{
    ProxyFiles, SECRET_HEADER, ServiceKind, base_url, healthy, install_plan, port_of,
    spawn_detached, uninstall_plan, wait_until,
};
use crate::env::Env;
use crate::io::Io;

/// Timeout of one health or admin request.
pub(super) const ADMIN_TIMEOUT: Duration = Duration::from_secs(2);
/// How long `start` and `stop` wait for the proxy to come up or go away.
pub const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// How long `start` and `stop` wait for the proxy to come up or go away.
    pub settle: Duration,
    /// Runs the proxy in this process until it stops (the foreground mode).
    pub serve_foreground: &'a dyn Fn(&str) -> Result<()>,
    /// Runs a system command (`systemctl`, `launchctl`, `schtasks`) for the user service.
    pub run_command: &'a dyn Fn(&[String]) -> Result<()>,
}

pub fn run(io: &mut Io, ctx: &ProxyCtx<'_>, command: &ProxyCommand) -> Result<()> {
    match command {
        ProxyCommand::Start { foreground, bind } => {
            require_enabled(ctx)?;
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
        ProxyCommand::Enable {
            port,
            record_content,
            yes,
        } => {
            let probe = |port| port::probe(&files(ctx.env), port);
            let given = Answers {
                port: *port,
                record_content: *record_content,
                ask: io.interactive() && !yes,
            };
            enable(io, ctx, given, &probe)
        }
        ProxyCommand::Disable => disable(io, ctx),
        ProxyCommand::Status { json } => status(io, ctx, *json),
        ProxyCommand::Service { command } => service(io, ctx, command),
    }
}

/// `proxy start` and the user service refuse to run while the proxy features are off.
fn require_enabled(ctx: &ProxyCtx<'_>) -> Result<()> {
    if ctx.config.proxy.enabled {
        return Ok(());
    }
    bail!(
        "the tokred proxy is disabled ([proxy] enabled = false in tokred.toml); run `tokred proxy enable` first"
    )
}

/// What `proxy enable` was told up front and whether it may ask for the rest.
#[derive(Debug, Clone, Copy)]
struct Answers {
    /// `--port` or `TOKRED_PORT`.
    port: Option<u16>,
    /// `--record-content` or `TOKRED_RECORD_CONTENT`.
    record_content: Option<bool>,
    /// A person is at the terminal and `--yes` was not given.
    ask: bool,
}

/// `proxy enable`: chooses the port and whether conversation content is recorded, saves them
/// with `[proxy] enabled = true` in `tokred.toml` (the user's other keys stay), then reconciles
/// Claude Code's settings at once: starts the proxy and points Claude Code at it. A proxy that
/// runs on another port is restarted on the new one.
///
/// A flag or environment value is used as given. Otherwise a person at the terminal is asked, and
/// without one the saved value stays (port 1029 and no recording when nothing was saved). `probe`
/// tells what listens on a port.
fn enable(
    io: &mut Io,
    ctx: &ProxyCtx<'_>,
    given: Answers,
    probe: &dyn Fn(u16) -> PortState,
) -> Result<()> {
    let port = choose_port(io, ctx, given, probe)?;
    let record = choose_content(io, ctx, given)?;
    let existing = std::fs::read_to_string(ctx.env.data.config_path()).unwrap_or_default();
    let bind = toml::Value::String(port::with_port(&ctx.config.proxy.bind, port));
    let text = set_key(&existing, "proxy", "bind", bind)?;
    let text = set_key(&text, "proxy", "log_bodies", toml::Value::Boolean(record))?;
    let text = set_key(&text, "proxy", "enabled", toml::Value::Boolean(true))?;
    let config = Config::from_toml_str(&text)?;

    // Saved before the restart: a proxy that is started again (by us, or by a user service that
    // restarts it when it exits) must find the new port in the file.
    save(ctx, &text)?;
    restart_if_stale(io, ctx, port, record)?;
    let content = if record {
        "recorded for the dashboard"
    } else {
        "not recorded"
    };
    let details = format!(
        "  address: {}\n  conversation content: {content}\n",
        config.proxy.bind
    );
    reconcile_and_report(io, ctx, &config, true, &details)
}

/// `proxy disable`: sets `[proxy] enabled = false`, then reconciles Claude Code's settings at once
/// (tokred's URL is removed). A running proxy is left running.
fn disable(io: &mut Io, ctx: &ProxyCtx<'_>) -> Result<()> {
    let existing = std::fs::read_to_string(ctx.env.data.config_path()).unwrap_or_default();
    let text = set_key(&existing, "proxy", "enabled", toml::Value::Boolean(false))?;
    save(ctx, &text)?;
    reconcile_and_report(io, ctx, ctx.config, false, "")
}

/// The port to use: the one given, else the answer of the person asked (the saved port
/// is the default), else the saved port, else 1029. A port given up front that is taken only
/// warns: the proxy starts once it is free.
fn choose_port(
    io: &mut Io,
    ctx: &ProxyCtx<'_>,
    given: Answers,
    probe: &dyn Fn(u16) -> PortState,
) -> Result<u16> {
    let saved = port_of(&ctx.config.proxy.bind).unwrap_or(port::DEFAULT_PORT);
    match given.port {
        Some(port) => port::warn_if_busy(io, probe, port).map(|()| port),
        None if given.ask => port::ask(io, probe, saved),
        None => Ok(saved),
    }
}

/// Whether conversation content is recorded: the answer given, else the person's (the saved
/// value is the default, yes when nothing was chosen yet), else the saved value, else no.
fn choose_content(io: &mut Io, ctx: &ProxyCtx<'_>, given: Answers) -> Result<bool> {
    let saved = saved_content_choice(&ctx.config.proxy);
    match given.record_content {
        Some(record) => Ok(record),
        None if given.ask => ask_record_content(io, saved.unwrap_or(true)),
        None => Ok(saved.unwrap_or(false)),
    }
}

/// What the user chose before about recording content, when `tokred.toml` tells. The file always
/// holds a `log_bodies` value (`configure` writes the default, `false`), so a `false` counts as a
/// choice only once the proxy has been enabled.
fn saved_content_choice(proxy: &ProxyConfig) -> Option<bool> {
    (proxy.enabled || proxy.log_bodies).then_some(proxy.log_bodies)
}

/// Explains and asks whether the dashboard may show what was sent and returned.
fn ask_record_content(io: &mut Io, default: bool) -> Result<bool> {
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

/// The port a proxy of this installation answers on and whether it records conversation content
/// (`None` when it does not say: a proxy of an older version). `None` when none is running.
///
/// The proxy is looked for where its pid file says, else on the configured port (the file can be
/// missing, for example after a restart; `stop` does the same). A proxy found only by the port
/// counts only if it answers with this installation's secret: it may be someone else's.
fn running_proxy(ctx: &ProxyCtx<'_>) -> Option<(u16, Option<bool>)> {
    let files = files(ctx.env);
    let recorded_port = files.read_pid().map(|(_, port)| port);
    let port = recorded_port.or_else(|| port_of(&ctx.config.proxy.bind).ok())?;
    if !healthy(port, ADMIN_TIMEOUT) {
        return None;
    }
    let status = files
        .secret()
        .ok()
        .and_then(|secret| admin(port, &secret, "GET", "/__tokred/status").ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    // Only a proxy that is on the record may be restarted without having proved it is ours.
    if status.is_none() && recorded_port.is_none() {
        return None;
    }
    let recorded = status.and_then(|status| status["content_recorded"].as_bool());
    Some((port, recorded))
}

/// Stops a proxy of this installation that does not match what was asked for (another port, or
/// another answer about recording conversation content: it reads its settings only when it
/// starts), so that the reconcile starts one that does.
fn restart_if_stale(io: &mut Io, ctx: &ProxyCtx<'_>, port: u16, record: bool) -> Result<()> {
    let Some((running, recorded)) = running_proxy(ctx) else {
        return Ok(());
    };
    let wanted = if record { "recorded" } else { "not recorded" };
    let why = if running != port {
        format!("is running on port {running}; restarting it on port {port}")
    } else if recorded != Some(record) {
        format!("has to restart to apply the setting; conversation content will be {wanted}")
    } else {
        return Ok(());
    };
    writeln!(io.out(), "The tokred proxy {why}.")?;
    stop(io, ctx)
}

/// Writes `text` as `tokred.toml`.
fn save(ctx: &ProxyCtx<'_>, text: &str) -> Result<()> {
    let path = ctx.env.data.config_path();
    std::fs::create_dir_all(ctx.env.data.root())
        .with_context(|| format!("could not create {}", ctx.env.data.root().display()))?;
    std::fs::write(&path, text).with_context(|| format!("could not write {}", path.display()))
}

/// Reconciles Claude Code's settings with `enabled` (using `config`, which is what `tokred.toml`
/// now says) and reports what happened. `details` are lines that follow the `[proxy] enabled`
/// line.
fn reconcile_and_report(
    io: &mut Io,
    ctx: &ProxyCtx<'_>,
    config: &Config,
    enabled: bool,
    details: &str,
) -> Result<()> {
    let report = initialize::reconcile(ctx.env, config, ctx.exe, enabled, ctx.settle)?;
    let mut shown = format!("[proxy] enabled = {enabled}\n{details}");
    for action in &report.actions {
        writeln!(shown, "  {action}")?;
    }
    let summary = match report.state {
        initialize::State::Enabled => format!(
            "The tokred proxy is on. Claude Code started from now on goes through {}.\n",
            base_url(port_of(&config.proxy.bind)?)
        ),
        initialize::State::Disabled => disabled_summary(service_state(ctx.env) == Some(true)),
        _ => "The setting is saved; see the warnings printed on stderr.\n".to_owned(),
    };
    write!(io.out(), "{shown}{summary}")?;
    write!(io.err(), "{}", report.warning_text())?;
    Ok(())
}

/// What `proxy disable` ends with. An installed user service would keep trying to start a proxy
/// that now refuses to run, so it is pointed out.
fn disabled_summary(service_installed: bool) -> String {
    let service = if service_installed {
        "The user service is still installed and will keep failing to start the proxy: remove it with `tokred proxy service uninstall`.\n"
    } else {
        ""
    };
    format!(
        "The proxy features are off. A running proxy keeps running until `tokred proxy stop`.\n{service}"
    )
}

/// Starts `tokred proxy start --foreground` in the background (detached, no standard streams),
/// creating the admin secret first because the child needs it. Does not wait for it.
pub fn spawn_proxy(env: &Env, exe: &Path, bind: &str) -> Result<()> {
    files(env).secret()?;
    spawn_detached(exe, &["proxy", "start", "--foreground", "--bind", bind]).map(|_| ())
}

fn start_background(io: &mut Io, ctx: &ProxyCtx<'_>, bind: &str) -> Result<()> {
    let port = port_of(bind)?;
    let url = base_url(port);
    if healthy(port, ADMIN_TIMEOUT) {
        writeln!(io.out(), "The tokred proxy is already running on {url}.")?;
        return Ok(());
    }
    spawn_proxy(ctx.env, ctx.exe, bind)?;
    if !wait_until(ctx.settle, Duration::from_millis(50), &mut || {
        healthy(port, ADMIN_TIMEOUT)
    }) {
        let logs = ctx.env.data.logs_dir();
        bail!(
            "the proxy did not answer on {url}; see the logs in {}",
            logs.display()
        );
    }
    let text = format!(
        "The tokred proxy is running on {url}.\nStart Claude Code through it with: tokred claude\n"
    );
    write!(io.out(), "{text}")?;
    Ok(())
}

pub(super) fn admin(port: u16, secret: &str, method: &str, path: &str) -> Result<String> {
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
    let stopping = files.read_pid().map(|(pid, _)| pid);
    admin(port, &files.secret()?, "POST", "/__tokred/shutdown")?;
    if !wait_until(ctx.settle, Duration::from_millis(50), &mut || {
        !healthy(port, ADMIN_TIMEOUT)
    }) {
        bail!("the proxy on port {port} did not stop");
    }
    stopping.map_or_else(|| files.clear_pid(), |pid| files.clear_pid_of(pid));
    writeln!(io.out(), "The tokred proxy stopped.")?;
    Ok(())
}

/// Whether the user service is installed, when it can be told: `None` where the service has no
/// file to look for (a Windows logon task) or the operating system has none.
pub fn service_state(env: &Env) -> Option<bool> {
    let kind = ServiceKind::for_os(std::env::consts::OS)?;
    let home = env.home_dir.as_deref()?;
    let (path, _) = install_plan(kind, Path::new("tokred"), home).file?;
    Some(path.exists())
}

/// Whether the user service of `kind` is installed: its file exists for this home folder, or, for
/// the Windows logon task (which has no file), the scheduler knows the task.
pub fn service_installed(kind: Option<ServiceKind>, ctx: &ProxyCtx<'_>) -> bool {
    match (kind, &ctx.env.home_dir) {
        (Some(ServiceKind::WindowsTask), _) => {
            (ctx.run_command)(&crate::daemon::task_query_command()).is_ok()
        }
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
    let base = ctx.env.anthropic_base_url.clone();
    let points_here = base
        .as_deref()
        .is_some_and(|url| url.trim_end_matches('/') == base_url(port));
    let kind = ServiceKind::for_os(std::env::consts::OS);
    let value = json!({
        "enabled": ctx.config.proxy.enabled,
        "running": running,
        "url": base_url(port),
        "pid": files.read_pid().map(|(pid, _)| pid),
        "service_installed": service_installed(kind, ctx),
        "anthropic_base_url": base,
        "anthropic_base_url_points_here": points_here,
        "routing_mode": ctx.config.routing.mode,
        "compression_mode": ctx.config.compression.mode,
        "network": tokred_proxy::NetworkSettings::from_env(ctx.config.proxy.ca_bundle.as_deref())
            .summary(&ctx.config.proxy.upstream),
        "details": details,
        "traffic": traffic_last_day(ctx.env),
    });
    if json {
        let text = serde_json::to_string_pretty(&value)?;
        writeln!(io.out(), "{text}")?;
        return Ok(());
    }
    let text = render_status(&value, running, points_here);
    write!(io.out(), "{text}")?;
    Ok(())
}

/// The human-readable `tokred proxy status` from the figures of the JSON form.
fn render_status(value: &Value, running: bool, points_here: bool) -> String {
    let state = match (running, value["enabled"] == true) {
        (true, _) => "running",
        (false, true) => "not running",
        (false, false) => "disabled",
    };
    let url = value["url"].as_str().unwrap_or("?");
    let service = if value["service_installed"] == true {
        "installed"
    } else {
        "not installed"
    };
    let terminal = if points_here {
        "ANTHROPIC_BASE_URL points to the proxy"
    } else {
        "ANTHROPIC_BASE_URL does not point to the proxy (use `tokred claude`)"
    };
    let routing = value["routing_mode"].as_str().unwrap_or("?");
    let compression = value["compression_mode"].as_str().unwrap_or("?");
    let traffic = &value["traffic"];
    let last_day = match (
        traffic["requests"].as_u64(),
        traffic["overhead_p50_ms"].as_f64(),
        traffic["overhead_p95_ms"].as_f64(),
    ) {
        (Some(n), Some(p50), Some(p95)) => {
            format!("{n} requests, proxy overhead p50 {p50:.2} ms, p95 {p95:.2} ms")
        }
        _ => "no requests recorded".to_owned(),
    };
    format!(
        "tokred proxy: {state} ({url})\n  user service:  {service}\n  this terminal: {terminal}\n  routing:       {routing}   compression: {compression}\n  last 24 hours: {last_day}\n"
    )
}

/// Requests recorded in the last 24 hours and the time the proxy itself added to them; `null`
/// when there is no database or no request yet.
fn traffic_last_day(env: &Env) -> Value {
    let now = SystemClock.now();
    existing_store(env)
        .ok()
        .flatten()
        .and_then(|store| {
            store
                .proxy_requests_between(Timestamp(now.0 - 86_400), Timestamp(now.0 + 1))
                .ok()
        })
        .map_or(Value::Null, |records| traffic_summary(&records))
}

/// The number of requests and the median and 95th percentile (nearest rank) of the overhead the
/// proxy added, in milliseconds; `null` for no requests.
fn traffic_summary(records: &[ProxyRequestRecord]) -> Value {
    let mut overhead: Vec<u64> = records.iter().map(|r| r.proxy_overhead_us).collect();
    overhead.sort_unstable();
    let rank = |percent: usize| overhead[(percent * overhead.len()).div_ceil(100) - 1];
    if overhead.is_empty() {
        return Value::Null;
    }
    #[allow(clippy::cast_precision_loss)]
    let ms = |us: u64| us as f64 / 1000.0;
    json!({
        "requests": overhead.len(),
        "overhead_p50_ms": ms(rank(50)),
        "overhead_p95_ms": ms(rank(95)),
    })
}

fn write_service_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    std::fs::write(path, content).with_context(|| format!("could not write {}", path.display()))
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
            require_enabled(ctx)?;
            let plan = install_plan(kind, ctx.exe, home);
            plan.file
                .iter()
                .try_for_each(|(path, content)| write_service_file(path, content))?;
            for command in &plan.commands {
                (ctx.run_command)(command)?;
            }
            let text = "Installed the tokred proxy as a user service; it starts at login.";
            writeln!(io.out(), "{text}")?;
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
        fake_proxy_with(secret, true)
    }

    /// With `obey` false the shutdown request is acknowledged but the proxy keeps running.
    fn fake_proxy_with(secret: String, obey: bool) -> (u16, Arc<AtomicBool>) {
        fake_proxy_saying(secret, obey, r#"{"requests":3,"content_recorded":false}"#)
    }

    /// A fake proxy whose status answer is `status`.
    fn fake_proxy_saying(
        secret: String,
        obey: bool,
        status: &'static str,
    ) -> (u16, Arc<AtomicBool>) {
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
                    (200, status.to_owned())
                } else if request.starts_with("post /__tokred/shutdown") && authorized {
                    flag.store(obey, Ordering::SeqCst);
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
            settle: Duration::from_millis(300),
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
        config.proxy.enabled = true;
        config.proxy.bind = format!("127.0.0.1:{port}");
        config
    }

    fn no_serve(_: &str) -> Result<()> {
        bail!("not in this test")
    }

    #[test]
    fn foreground_start_runs_the_proxy_in_this_process() {
        let sb = Sandbox::new();
        let config = config_on(1);
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
        let text = out.text();
        assert!(text.contains("already running"), "{text}");

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
        assert!(value["network"]["ca_files"].is_array(), "{value}");

        let (mut io, out) = scripted(false, "");
        run(
            &mut io,
            &ctx(&sb, &config, &no_serve),
            &ProxyCommand::Status { json: false },
        )
        .unwrap();
        let text = out.text();
        assert!(text.contains("tokred proxy: running"), "{text}");
        assert!(text.contains("routing:       observe"));

        let (mut io, out) = scripted(false, "");
        run(&mut io, &ctx(&sb, &config, &no_serve), &ProxyCommand::Stop).unwrap();
        let text = out.text();
        assert!(text.contains("stopped"), "{text}");
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
        let text = out.text();
        assert!(text.contains("tokred proxy: not running"), "{text}");
        assert!(text.contains("use `tokred claude`"));
    }

    #[test]
    fn the_service_is_installed_and_removed_through_the_command_runner() {
        // Linux, macOS and Windows are all supported.
        let kind = ServiceKind::for_os(std::env::consts::OS).unwrap();
        let sb = Sandbox::new();
        let config = config_on(1);
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
        plan.file.iter().for_each(|(path, content)| {
            assert_eq!(std::fs::read_to_string(path).unwrap(), *content);
        });
        assert_eq!(ran.borrow().len(), plan.commands.len());
        // The recording runner answers every command with success, so a logon task (no file)
        // counts as installed too.
        assert_eq!(
            service_installed(Some(kind), &ctx),
            plan.file.is_some() || kind == ServiceKind::WindowsTask
        );

        let (mut io, out) = scripted(false, "");
        let uninstall = ProxyCommand::Service {
            command: ServiceCommand::Uninstall,
        };
        run(&mut io, &ctx, &uninstall).unwrap();
        assert!(out.text().contains("Removed the tokred proxy user service"));
        plan.file
            .iter()
            .for_each(|(path, _)| assert!(!path.exists()));

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

    #[test]
    fn a_disabled_proxy_refuses_to_start_or_to_become_a_service() {
        let sb = Sandbox::new();
        let config = Config::default();
        assert!(!config.proxy.enabled);
        for command in [
            ProxyCommand::Start {
                foreground: true,
                bind: None,
            },
            ProxyCommand::Start {
                foreground: false,
                bind: None,
            },
            ProxyCommand::Service {
                command: ServiceCommand::Install,
            },
        ] {
            let (mut io, _) = scripted(false, "");
            let err = run(&mut io, &ctx(&sb, &config, &no_serve), &command).unwrap_err();
            let text = format!("{err:#}");
            assert!(
                text.contains("is disabled") && text.contains("tokred proxy enable"),
                "{text}"
            );
        }
        // Status and stop still work, and status says why nothing runs.
        let (mut io, out) = scripted(false, "");
        let status = ProxyCommand::Status { json: false };
        run(
            &mut io,
            &ctx(&sb, &config_off_on_a_free_port(), &no_serve),
            &status,
        )
        .unwrap();
        let text = out.text();
        assert!(text.contains("tokred proxy: disabled"), "{text}");
    }

    fn config_off_on_a_free_port() -> Config {
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        Config {
            proxy: tokred_core::config::ProxyConfig {
                enabled: false,
                ..config_on(closed).proxy
            },
            ..Config::default()
        }
    }

    /// A sandbox where `claude` would use the sandbox's Claude folder.
    fn claude_sandbox(settings: &str) -> Sandbox {
        let mut sb = Sandbox::new();
        sb.env.claude_config_dir = Some(sb.claude.clone());
        sb.write_settings(settings);
        sb
    }

    fn settings_text(sb: &Sandbox) -> String {
        std::fs::read_to_string(sb.settings()).unwrap()
    }

    fn enabled_in_file(sb: &Sandbox) -> bool {
        let text = std::fs::read_to_string(sb.env.data.config_path()).unwrap();
        Config::from_toml_str(&text).unwrap().proxy.enabled
    }

    #[test]
    fn enable_and_disable_set_the_switch_and_reconcile_at_once() {
        let sb = claude_sandbox(r#"{"model":"opus"}"#);
        let (port, _) = fake_proxy(files(&sb.env).secret().unwrap());
        let config = config_on(port);
        let (mut io, out, err) = crate::io::testing::scripted_with_err(false, "");
        run(&mut io, &ctx(&sb, &config, &no_serve), &enable_command()).unwrap();
        let text = out.text();
        assert!(text.contains("[proxy] enabled = true"), "{text}");
        assert!(
            text.contains(&format!("goes through http://127.0.0.1:{port}")),
            "{text}"
        );
        assert_eq!(err.text(), "");
        assert!(enabled_in_file(&sb));
        assert!(settings_text(&sb).contains(&format!("http://127.0.0.1:{port}")));

        let (mut io, out, _) = crate::io::testing::scripted_with_err(false, "");
        run(
            &mut io,
            &ctx(&sb, &config, &no_serve),
            &ProxyCommand::Disable,
        )
        .unwrap();
        let text = out.text();
        assert!(text.contains("[proxy] enabled = false"), "{text}");
        assert!(
            text.contains("keeps running until `tokred proxy stop`"),
            "{text}"
        );
        assert!(!enabled_in_file(&sb));
        assert!(!settings_text(&sb).contains("ANTHROPIC_BASE_URL"));
        assert!(settings_text(&sb).contains("opus"));
    }

    #[test]
    fn disabling_with_a_user_service_installed_says_to_remove_it() {
        assert!(!disabled_summary(false).contains("service"));
        assert!(disabled_summary(true).contains("tokred proxy service uninstall"));

        let sb = claude_sandbox("{}");
        let config = config_on(1);
        let kind = ServiceKind::for_os(std::env::consts::OS).unwrap();
        let home = sb.env.home_dir.clone().unwrap();
        // Where the service has a file (not on Windows), put one in place.
        let plan = install_plan(kind, Path::new("tokred"), &home);
        plan.file.iter().for_each(|(path, content)| {
            write_service_file(path, content).unwrap();
        });
        let (mut io, out) = scripted(false, "");
        let disable = ProxyCommand::Disable;
        run(&mut io, &ctx(&sb, &config, &no_serve), &disable).unwrap();
        let text = out.text();
        // Where the service has a file (not on Windows), it is pointed out.
        assert_eq!(
            text.contains("tokred proxy service uninstall"),
            plan.file.is_some(),
            "{text}"
        );
    }

    #[test]
    fn enable_saves_the_switch_even_when_the_proxy_cannot_start_or_another_gateway_is_set() {
        let down = config_on(1);
        let sb = claude_sandbox("{}");
        let (mut io, out, err) = crate::io::testing::scripted_with_err(false, "");
        run(&mut io, &ctx(&sb, &down, &no_serve), &enable_command()).unwrap();
        let (text, warnings) = (out.text(), err.text());
        assert!(text.contains("The setting is saved"), "{text}");
        assert!(warnings.contains("could not be started"), "{warnings}");
        assert!(enabled_in_file(&sb));
        assert!(!settings_text(&sb).contains("ANTHROPIC_BASE_URL"));

        let sb = claude_sandbox(r#"{"env":{"ANTHROPIC_BASE_URL":"https://gateway.example"}}"#);
        let (mut io, _, err) = crate::io::testing::scripted_with_err(false, "");
        run(&mut io, &ctx(&sb, &down, &no_serve), &enable_command()).unwrap();
        let warnings = err.text();
        assert!(warnings.contains("https://gateway.example"), "{warnings}");
        assert!(settings_text(&sb).contains("https://gateway.example"));
    }

    #[test]
    fn enable_rejects_a_config_file_it_cannot_edit() {
        let sb = claude_sandbox("{}");
        std::fs::create_dir_all(sb.env.data.root()).unwrap();
        std::fs::write(sb.env.data.config_path(), "[[[").unwrap();
        let (mut io, _) = scripted(false, "");
        let result = run(
            &mut io,
            &ctx(&sb, &config_on(1), &no_serve),
            &enable_command(),
        );
        assert!(result.is_err());
    }

    fn enable_command() -> ProxyCommand {
        ProxyCommand::Enable {
            port: None,
            record_content: None,
            yes: false,
        }
    }

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// `proxy enable` with a fake port probe (a proxy that cannot start is fine: what these
    /// tests look at is what was asked and saved).
    fn enable_with(
        sb: &Sandbox,
        config: &Config,
        interactive: bool,
        input: &str,
        given: (Option<u16>, Option<bool>),
        probe: &dyn Fn(u16) -> PortState,
    ) -> (Result<()>, String) {
        let (mut io, out) = scripted(interactive, input);
        let answers = Answers {
            port: given.0,
            record_content: given.1,
            ask: interactive,
        };
        let result = enable(&mut io, &ctx(sb, config, &no_serve), answers, probe);
        (result, out.text())
    }

    fn saved(sb: &Sandbox) -> Config {
        let text = std::fs::read_to_string(sb.env.data.config_path()).unwrap();
        Config::from_toml_str(&text).unwrap()
    }

    fn all_free(_: u16) -> PortState {
        PortState::Free
    }

    fn fresh_config() -> Config {
        Config::default()
    }

    #[test]
    fn a_person_is_asked_for_the_port_and_for_recording_with_these_defaults() {
        let sb = claude_sandbox("{}");
        let (result, out) =
            enable_with(&sb, &fresh_config(), true, "\n\n", (None, None), &all_free);
        result.unwrap();
        assert!(out.contains("Port for the tokred proxy [1029]: "), "{out}");
        assert!(out.contains("nothing leaves this computer"), "{out}");
        assert!(out.contains("show it? [Y/n] "), "{out}");
        assert!(out.contains("address: 127.0.0.1:1029"), "{out}");
        assert!(
            out.contains("conversation content: recorded for the dashboard"),
            "{out}"
        );
        let config = saved(&sb);
        assert!(config.proxy.enabled);
        assert_eq!(config.proxy.bind, "127.0.0.1:1029");
        assert!(config.proxy.log_bodies);
    }

    #[test]
    fn the_answers_are_saved_and_become_the_next_defaults() {
        let sb = claude_sandbox("{}");
        let (result, out) = enable_with(
            &sb,
            &fresh_config(),
            true,
            "2000\nn\n",
            (None, None),
            &all_free,
        );
        result.unwrap();
        assert!(out.contains("conversation content: not recorded"), "{out}");
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "127.0.0.1:2000");
        assert!(!config.proxy.log_bodies);
        assert!(config.proxy.enabled);

        // Asked again, the saved values are the defaults: pressing Enter keeps them.
        let (result, out) = enable_with(&sb, &config, true, "\n\n", (None, None), &all_free);
        result.unwrap();
        assert!(out.contains("Port for the tokred proxy [2000]: "), "{out}");
        assert!(out.contains("show it? [y/N] "), "{out}");
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "127.0.0.1:2000");
        assert!(!config.proxy.log_bodies);

        let (result, _) = enable_with(&sb, &config, true, "\nYES\n", (None, None), &all_free);
        result.unwrap();
        assert!(saved(&sb).proxy.log_bodies);
    }

    #[test]
    fn a_content_default_that_was_never_chosen_is_yes() {
        // `configure` leaves `log_bodies = false` behind: that is not a choice while the proxy
        // has never been enabled, but it is one once it has.
        let off = ProxyConfig::default();
        assert_eq!(saved_content_choice(&off), None);
        let chosen_no = ProxyConfig {
            enabled: true,
            ..off.clone()
        };
        assert_eq!(saved_content_choice(&chosen_no), Some(false));
        let yes = ProxyConfig {
            log_bodies: true,
            ..off
        };
        assert_eq!(saved_content_choice(&yes), Some(true));
    }

    #[test]
    fn a_bad_or_busy_port_is_asked_again_and_a_port_that_is_ours_is_accepted() {
        let sb = claude_sandbox("{}");
        let busy = |port: u16| {
            if port == 3000 {
                PortState::Busy
            } else {
                PortState::Free
            }
        };
        let input = "abc\n80\n3000\n3001\nn\n";
        let (result, out) = enable_with(&sb, &fresh_config(), true, input, (None, None), &busy);
        result.unwrap();
        assert!(out.contains("`abc` is not a port number"), "{out}");
        assert!(
            out.contains("Port 3000 is in use by another program. Choose another."),
            "{out}"
        );
        assert_eq!(saved(&sb).proxy.bind, "127.0.0.1:3001");

        let ours = |_| PortState::Ours;
        let (result, _) = enable_with(&sb, &fresh_config(), true, "4000\nn\n", (None, None), &ours);
        result.unwrap();
        assert_eq!(saved(&sb).proxy.bind, "127.0.0.1:4000");
    }

    #[test]
    fn flags_answer_for_the_user_and_a_busy_flag_port_only_warns() {
        let sb = claude_sandbox("{}");
        let busy = |_| PortState::Busy;
        let given = (Some(5000), Some(true));
        let (result, out) = enable_with(&sb, &fresh_config(), true, "", given, &busy);
        result.unwrap();
        assert!(
            out.contains("Port 5000 is in use by another program: the proxy cannot start"),
            "{out}"
        );
        assert!(!out.contains("Port for the tokred proxy"), "{out}");
        assert!(!out.contains("Record conversation"), "{out}");
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "127.0.0.1:5000");
        assert!(config.proxy.log_bodies);
        assert!(config.proxy.enabled);
    }

    #[test]
    fn a_flag_for_one_answer_still_asks_the_other() {
        let sb = claude_sandbox("{}");
        let (result, out) = enable_with(
            &sb,
            &fresh_config(),
            true,
            "n\n",
            (Some(5001), None),
            &all_free,
        );
        result.unwrap();
        assert!(!out.contains("Port for the tokred proxy"), "{out}");
        assert!(out.contains("show it? [Y/n] "), "{out}");
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "127.0.0.1:5001");
        assert!(!config.proxy.log_bodies);
    }

    #[test]
    fn without_a_terminal_only_the_given_answers_are_written() {
        // Nothing saved, nothing said: port 1029, content not recorded.
        let sb = claude_sandbox("{}");
        let (result, out) = enable_with(&sb, &fresh_config(), false, "", (None, None), &all_free);
        result.unwrap();
        assert!(!out.contains("Port for the tokred proxy"), "{out}");
        assert!(!out.contains("Record conversation"), "{out}");
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "127.0.0.1:1029");
        assert!(!config.proxy.log_bodies);
        assert!(config.proxy.enabled);

        // What the user chose before stays (a re-run after an upgrade changes nothing)...
        let mut chosen = fresh_config();
        chosen.proxy.enabled = true;
        chosen.proxy.bind = "127.0.0.1:2500".to_owned();
        chosen.proxy.log_bodies = true;
        let (result, _) = enable_with(&sb, &chosen, false, "", (None, None), &all_free);
        result.unwrap();
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "127.0.0.1:2500");
        assert!(config.proxy.log_bodies);

        // ... unless a flag says otherwise.
        let (result, _) = enable_with(&sb, &chosen, false, "", (None, Some(false)), &all_free);
        result.unwrap();
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "127.0.0.1:2500");
        assert!(!config.proxy.log_bodies);
    }

    #[test]
    fn the_host_of_a_custom_bind_and_the_users_other_keys_survive() {
        let sb = claude_sandbox("{}");
        std::fs::create_dir_all(sb.env.data.root()).unwrap();
        let before = "[proxy]\nbind = \"0.0.0.0:7000\"\nlog_bodies = true\nretention_days = 5\n\n[routing]\nmode = \"enforce\"\n";
        std::fs::write(sb.env.data.config_path(), before).unwrap();
        let config = saved(&sb);
        let (result, out) = enable_with(&sb, &config, true, "\n\n", (None, None), &all_free);
        result.unwrap();
        assert!(out.contains("Port for the tokred proxy [7000]: "), "{out}");
        assert!(out.contains("show it? [Y/n] "), "{out}");
        let (result, _) = enable_with(&sb, &config, false, "", (Some(7100), None), &all_free);
        result.unwrap();
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, "0.0.0.0:7100");
        assert_eq!(config.proxy.retention_days, 5);
        assert!(config.proxy.log_bodies);
        assert_eq!(config.routing.mode.as_str(), "enforce");
    }

    #[test]
    fn a_proxy_running_on_another_port_is_restarted_on_the_new_one() {
        let sb = claude_sandbox("{}");
        let (old, stopped) = fake_proxy(files(&sb.env).secret().unwrap());
        files(&sb.env).write_pid(1234, old).unwrap();
        let config = config_on(old);
        let new = free_port();
        let given = (Some(new), Some(false));
        let (result, out) = enable_with(&sb, &config, false, "", given, &all_free);
        result.unwrap();
        assert!(
            out.contains(&format!(
                "running on port {old}; restarting it on port {new}"
            )),
            "{out}"
        );
        assert!(out.contains("The tokred proxy stopped."), "{out}");
        assert!(stopped.load(Ordering::SeqCst));
        assert_eq!(saved(&sb).proxy.bind, format!("127.0.0.1:{new}"));
    }

    #[test]
    fn a_running_proxy_is_restarted_when_the_content_setting_changes() {
        // The bug: `--record-content false` was saved, but the running proxy kept recording.
        for (says, record, restarts) in [
            (r#"{"content_recorded":true}"#, false, true),
            (r#"{"content_recorded":false}"#, true, true),
            // An older proxy does not say what it records: restart, to be sure.
            (r#"{"requests":3}"#, false, true),
            (r#"{"content_recorded":true}"#, true, false),
            (r#"{"content_recorded":false}"#, false, false),
        ] {
            let sb = claude_sandbox("{}");
            let (port, stopped) = fake_proxy_saying(files(&sb.env).secret().unwrap(), true, says);
            files(&sb.env).write_pid(1234, port).unwrap();
            let (result, out) = enable_with(
                &sb,
                &config_on(port),
                false,
                "",
                (Some(port), Some(record)),
                &|_| PortState::Ours,
            );
            result.unwrap();
            assert_eq!(
                stopped.load(Ordering::SeqCst),
                restarts,
                "{says} {record}: {out}"
            );
            assert_eq!(
                out.contains("restart to apply the setting"),
                restarts,
                "{out}"
            );
            if restarts {
                let wanted = if record {
                    "will be recorded"
                } else {
                    "will be not recorded"
                };
                assert!(out.contains(wanted), "{out}");
            }
        }
    }

    #[test]
    fn a_running_proxy_is_found_by_its_port_when_the_pid_file_is_gone() {
        // The pid file can be missing (an old proxy that ended late used to erase it). The proxy
        // that answers with this installation's secret on the configured port is the one.
        let sb = claude_sandbox("{}");
        let (port, stopped) = fake_proxy_saying(
            files(&sb.env).secret().unwrap(),
            true,
            r#"{"content_recorded":true}"#,
        );
        assert_eq!(files(&sb.env).read_pid(), None);
        let (result, out) = enable_with(
            &sb,
            &config_on(port),
            false,
            "",
            (Some(port), Some(false)),
            &|_| PortState::Ours,
        );
        result.unwrap();
        assert!(stopped.load(Ordering::SeqCst), "{out}");
        assert!(out.contains("restart to apply the setting"), "{out}");
    }

    #[test]
    fn a_proxy_of_another_installation_on_the_port_is_left_alone() {
        let sb = claude_sandbox("{}");
        // It answers health, but not with this installation's secret.
        let (port, stopped) = fake_proxy_saying(
            "someone else's secret".to_owned(),
            true,
            r#"{"content_recorded":true}"#,
        );
        let (result, out) = enable_with(
            &sb,
            &config_on(port),
            false,
            "",
            (Some(port), Some(false)),
            &|_| PortState::Ours,
        );
        result.unwrap();
        assert!(!stopped.load(Ordering::SeqCst), "{out}");
        assert!(!out.contains("restart"), "{out}");
    }

    #[test]
    fn the_new_port_is_saved_before_the_old_proxy_is_stopped() {
        let sb = claude_sandbox("{}");
        // A proxy that acknowledges the shutdown but keeps running: the stop fails.
        let (old, _) = fake_proxy_with(files(&sb.env).secret().unwrap(), false);
        files(&sb.env).write_pid(1234, old).unwrap();
        let new = free_port();
        let given = (Some(new), Some(false));
        let (result, _) = enable_with(&sb, &config_on(old), false, "", given, &all_free);
        assert!(format!("{:#}", result.unwrap_err()).contains("did not stop"));
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, format!("127.0.0.1:{new}"));
        assert!(config.proxy.enabled);
    }

    #[test]
    fn a_proxy_already_on_the_chosen_port_is_not_restarted() {
        let sb = claude_sandbox("{}");
        let (port, stopped) = fake_proxy(files(&sb.env).secret().unwrap());
        files(&sb.env).write_pid(1234, port).unwrap();
        let (result, out) = enable_with(
            &sb,
            &config_on(port),
            false,
            "",
            (Some(port), None),
            &|_| PortState::Ours,
        );
        result.unwrap();
        assert!(!out.contains("restarting"), "{out}");
        assert!(!stopped.load(Ordering::SeqCst));
        assert!(out.contains("The tokred proxy is on."), "{out}");
    }

    #[test]
    fn a_recorded_proxy_that_is_gone_is_not_stopped() {
        let sb = claude_sandbox("{}");
        let gone = free_port();
        files(&sb.env).write_pid(1234, gone).unwrap();
        let (result, out) = enable_with(
            &sb,
            &fresh_config(),
            false,
            "",
            (Some(free_port()), None),
            &all_free,
        );
        result.unwrap();
        assert!(!out.contains("restarting"), "{out}");
    }

    #[test]
    fn the_command_takes_flags_and_the_real_port_probe() {
        let sb = claude_sandbox("{}");
        let port = free_port();
        let command = ProxyCommand::Enable {
            port: Some(port),
            record_content: Some(true),
            yes: false,
        };
        let (mut io, out) = scripted(false, "");
        run(&mut io, &ctx(&sb, &fresh_config(), &no_serve), &command).unwrap();
        let text = out.text();
        assert!(
            text.contains(&format!("address: 127.0.0.1:{port}")),
            "{text}"
        );
        assert!(!text.contains("in use by another program"), "{text}");
        let config = saved(&sb);
        assert_eq!(config.proxy.bind, format!("127.0.0.1:{port}"));
        assert!(config.proxy.log_bodies && config.proxy.enabled);

        // A port somebody else holds is only a warning.
        let held = TcpListener::bind("127.0.0.1:0").unwrap();
        let busy = held.local_addr().unwrap().port();
        let command = ProxyCommand::Enable {
            port: Some(busy),
            record_content: None,
            yes: false,
        };
        let (mut io, out) = scripted(false, "");
        run(&mut io, &ctx(&sb, &config, &no_serve), &command).unwrap();
        let text = out.text();
        assert!(
            text.contains(&format!("Port {busy} is in use by another program")),
            "{text}"
        );
    }

    #[test]
    fn yes_never_asks_even_in_a_terminal() {
        let sb = claude_sandbox("{}");
        let port = free_port();
        let command = ProxyCommand::Enable {
            port: None,
            record_content: None,
            yes: true,
        };
        let (mut io, out) = scripted(true, "should never be read\n");
        run(&mut io, &ctx(&sb, &config_on(port), &no_serve), &command).unwrap();
        let text = out.text();
        assert!(!text.contains("Port for the tokred proxy"), "{text}");
        assert!(!text.contains("Record conversation"), "{text}");
        assert_eq!(saved(&sb).proxy.bind, format!("127.0.0.1:{port}"));
    }

    #[test]
    fn enable_refuses_a_config_that_is_not_toml_before_asking_to_stop_anything() {
        let sb = claude_sandbox("{}");
        std::fs::create_dir_all(sb.env.data.root()).unwrap();
        std::fs::write(sb.env.data.config_path(), "[[[").unwrap();
        let (result, _) = enable_with(&sb, &fresh_config(), false, "", (None, None), &all_free);
        assert!(format!("{:#}", result.unwrap_err()).contains("not valid TOML"));
    }

    fn ctx_with_refusal<'a>(sb: &'a Sandbox, config: &'a Config) -> ProxyCtx<'a> {
        ctx(sb, config, &no_serve)
    }

    #[test]
    fn the_admin_endpoints_refuse_a_wrong_secret_and_the_placeholder_serve_fails() {
        let sb = Sandbox::new();
        let (port, _) = fake_proxy(files(&sb.env).secret().unwrap());
        let err = admin(port, "wrong", "GET", "/__tokred/status").unwrap_err();
        assert!(err.to_string().contains("refused"), "{err}");
        assert!(no_serve("127.0.0.1:1").is_err());
    }

    fn record_with_overhead(id: &str, overhead_us: u64) -> ProxyRequestRecord {
        ProxyRequestRecord {
            id: id.to_owned(),
            proxy_overhead_us: overhead_us,
            ..ProxyRequestRecord::default()
        }
    }

    #[test]
    fn the_traffic_summary_uses_nearest_rank_percentiles() {
        assert_eq!(traffic_summary(&[]), Value::Null);
        let one = traffic_summary(&[record_with_overhead("a", 1500)]);
        assert_eq!(one["requests"], 1);
        assert_eq!(one["overhead_p50_ms"], 1.5);
        assert_eq!(one["overhead_p95_ms"], 1.5);
        let many: Vec<_> = (1..=20)
            .map(|n| record_with_overhead(&format!("r{n}"), n * 1000))
            .collect();
        let summary = traffic_summary(&many);
        assert_eq!(summary["requests"], 20);
        assert_eq!(summary["overhead_p50_ms"], 10.0);
        assert_eq!(summary["overhead_p95_ms"], 19.0);
    }

    #[test]
    fn the_last_day_comes_from_the_database_and_is_empty_without_one() {
        let bare = Sandbox::new();
        assert_eq!(traffic_last_day(&bare.env), Value::Null);

        let sb = Sandbox::new();
        sb.configure();
        assert_eq!(traffic_last_day(&sb.env), Value::Null, "no requests yet");
        let now = SystemClock.now();
        let recent = ProxyRequestRecord {
            ts: Timestamp(now.0 - 60),
            ..record_with_overhead("recent", 2000)
        };
        let old = ProxyRequestRecord {
            ts: Timestamp(now.0 - 3 * 86_400),
            ..record_with_overhead("old", 9000)
        };
        sb.store().insert_proxy_request(&recent).unwrap();
        sb.store().insert_proxy_request(&old).unwrap();
        let day = traffic_last_day(&sb.env);
        assert_eq!(day["requests"], 1, "only the last 24 hours: {day}");
        assert_eq!(day["overhead_p95_ms"], 2.0);
    }

    #[test]
    fn the_status_text_names_the_service_and_where_this_terminal_points() {
        let value = json!({
            "url": "http://127.0.0.1:1029",
            "service_installed": true,
            "routing_mode": "enforce",
            "compression_mode": "observe",
        });
        let text = render_status(&value, true, true);
        assert!(
            text.contains("last 24 hours: no requests recorded"),
            "{text}"
        );
        let busy = json!({
            "traffic": {"requests": 4, "overhead_p50_ms": 0.4, "overhead_p95_ms": 1.25},
        });
        let text = render_status(&busy, true, false);
        assert!(
            text.contains("4 requests, proxy overhead p50 0.40 ms, p95 1.25 ms"),
            "{text}"
        );
        let text = render_status(&value, true, true);
        assert!(
            text.contains("tokred proxy: running (http://127.0.0.1:1029)"),
            "{text}"
        );
        assert!(text.contains("user service:  installed"), "{text}");
        assert!(
            text.contains("ANTHROPIC_BASE_URL points to the proxy"),
            "{text}"
        );
        assert!(
            text.contains("routing:       enforce   compression: observe"),
            "{text}"
        );
    }

    #[test]
    fn a_proxy_that_never_answers_is_reported_with_the_logs_folder() {
        let sb = Sandbox::new();
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let config = config_on(closed);
        // The test binary starts (and exits at once, not understanding the arguments), so the
        // spawn works but nothing ever listens.
        let exe = std::env::current_exe().unwrap();
        let ctx = ProxyCtx {
            exe: &exe,
            settle: Duration::from_millis(200),
            ..ctx(&sb, &config, &no_serve)
        };
        let command = ProxyCommand::Start {
            foreground: false,
            bind: None,
        };
        let (mut io, _) = scripted(false, "");
        let err = run(&mut io, &ctx, &command).unwrap_err();
        assert!(format!("{err:#}").contains("did not answer"), "{err:#}");
    }

    #[test]
    fn a_proxy_that_ignores_shutdown_is_reported() {
        let sb = Sandbox::new();
        let secret = files(&sb.env).secret().unwrap();
        let (port, _) = fake_proxy_with(secret, false);
        let config = config_on(port);
        let ctx = ProxyCtx {
            settle: Duration::from_millis(200),
            ..ctx(&sb, &config, &no_serve)
        };
        let (mut io, _) = scripted(false, "");
        let err = run(&mut io, &ctx, &ProxyCommand::Stop).unwrap_err();
        assert!(err.to_string().contains("did not stop"), "{err}");
    }

    #[test]
    fn a_windows_logon_task_is_found_by_asking_the_scheduler() {
        let sb = Sandbox::new();
        let config = Config::default();
        let asked = std::cell::RefCell::new(Vec::new());
        let present = |command: &[String]| -> Result<()> {
            asked.borrow_mut().push(command.join(" "));
            Ok(())
        };
        let ctx_present = ProxyCtx {
            run_command: &present,
            ..ctx(&sb, &config, &no_serve)
        };
        assert!(service_installed(
            Some(ServiceKind::WindowsTask),
            &ctx_present
        ));
        assert_eq!(*asked.borrow(), ["schtasks /Query /TN tokred-proxy"]);
        // The default test runner refuses every command: the task is not there.
        let ctx_absent = ctx(&sb, &config, &no_serve);
        assert!(!service_installed(
            Some(ServiceKind::WindowsTask),
            &ctx_absent
        ));
    }

    #[test]
    fn service_files_report_which_step_failed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, "x").unwrap();
        // The parent is a regular file: it cannot become a folder.
        let err = write_service_file(&file.join("unit"), "x").unwrap_err();
        assert!(err.to_string().contains("could not create"), "{err}");
        // The target is a folder: it cannot be written as a file.
        let err = write_service_file(dir.path(), "x").unwrap_err();
        assert!(err.to_string().contains("could not write"), "{err}");
        // No parent at all: only the write is attempted, and it fails.
        let err = write_service_file(Path::new(""), "x").unwrap_err();
        assert!(err.to_string().contains("could not write"), "{err}");
        let ok = dir.path().join("nested").join("unit");
        write_service_file(&ok, "content").unwrap();
        assert_eq!(std::fs::read_to_string(ok).unwrap(), "content");
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
