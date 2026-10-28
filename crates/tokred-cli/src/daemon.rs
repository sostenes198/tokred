//! Running the proxy in the background: its admin secret and pid files, health checks, starting it
//! detached, and the user-service definitions (systemd user unit, launchd agent, Windows logon
//! task). The proxy itself lives in `tokred-proxy`; this module only manages the process.

use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// Header the proxy's admin endpoints require.
pub const SECRET_HEADER: &str = "x-tokred-secret";
/// Name of the user service.
pub const SERVICE_NAME: &str = "tokred-proxy";

/// Files of the running proxy, under `<data>/proxy/`.
#[derive(Debug, Clone)]
pub struct ProxyFiles {
    dir: PathBuf,
}

impl ProxyFiles {
    /// The files under `dir` (usually `<data>/proxy`).
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The admin secret file.
    #[must_use]
    pub fn secret_path(&self) -> PathBuf {
        self.dir.join("secret")
    }

    /// The pid file (`<pid> <port>`).
    #[must_use]
    pub fn pid_path(&self) -> PathBuf {
        self.dir.join("pid")
    }

    /// Reads the admin secret, creating it (owner-only permissions) on first use.
    pub fn secret(&self) -> Result<String> {
        let path = self.secret_path();
        if let Ok(text) = std::fs::read_to_string(&path) {
            let text = text.trim().to_owned();
            if text.len() >= 32 {
                return Ok(text);
            }
        }
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("could not create {}", self.dir.display()))?;
        let secret = random_hex();
        std::fs::write(&path, &secret)
            .with_context(|| format!("could not write {}", path.display()))?;
        restrict_to_owner(&path)?;
        Ok(secret)
    }

    /// Records the running proxy.
    pub fn write_pid(&self, pid: u32, port: u16) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("could not create {}", self.dir.display()))?;
        let path = self.pid_path();
        std::fs::write(&path, format!("{pid} {port}\n"))
            .with_context(|| format!("could not write {}", path.display()))
    }

    /// The recorded `(pid, port)`, if the file exists and is well formed.
    #[must_use]
    pub fn read_pid(&self) -> Option<(u32, u16)> {
        parse_pid(&std::fs::read_to_string(self.pid_path()).ok()?)
    }

    /// Forgets the running proxy.
    pub fn clear_pid(&self) {
        let _ = std::fs::remove_file(self.pid_path());
    }
}

fn parse_pid(text: &str) -> Option<(u32, u16)> {
    let mut parts = text.split_whitespace();
    let pid = parts.next()?.parse().ok()?;
    let port = parts.next()?.parse().ok()?;
    Some((pid, port))
}

/// 256 random bits as hex. `RandomState` is seeded from the operating system's randomness, so
/// this needs no extra dependency; the secret only guards a loopback admin endpoint.
fn random_hex() -> String {
    (0..4).fold(String::with_capacity(64), |mut out, round| {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(round);
        let _ = write!(out, "{:016x}", hasher.finish());
        out
    })
}

#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("could not restrict {}", path.display()))
}

#[cfg(not(unix))]
#[allow(clippy::unnecessary_wraps)] // same signature as the unix version
fn restrict_to_owner(_path: &Path) -> Result<()> {
    // The data folder under %APPDATA% is already private to the user.
    Ok(())
}

/// The URL Claude Code is pointed at.
#[must_use]
pub fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// The port of a `host:port` bind address.
pub fn port_of(bind: &str) -> Result<u16> {
    bind.rsplit_once(':')
        .and_then(|(_, port)| port.parse().ok())
        .with_context(|| format!("`{bind}` is not a host:port address"))
}

/// Whether a tokred proxy answers its health endpoint on `port`.
#[must_use]
pub fn healthy(port: u16, timeout: Duration) -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into();
    agent
        .get(format!("{}/__tokred/health", base_url(port)))
        .call()
        .ok()
        .and_then(|mut response| response.body_mut().read_to_string().ok())
        .is_some_and(|body| body.contains("\"ok\":true"))
}

/// Calls `check` until it succeeds or `timeout` passes.
pub fn wait_until(timeout: Duration, step: Duration, check: &mut dyn FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if check() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(step);
    }
}

/// Starts `exe args...` detached from this terminal (its own process group on Unix, a detached
/// process on Windows), with no standard streams. Returns its pid.
pub fn spawn_detached(exe: &Path, args: &[&str]) -> Result<u32> {
    let mut command = Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut command);
    let child = command
        .spawn()
        .with_context(|| format!("could not start {}", exe.display()))?;
    Ok(child.id())
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // A process group of its own: closing the terminal does not take the proxy down.
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

/// How the proxy is installed as a user service on this operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    /// A systemd user unit (Linux).
    Systemd,
    /// A launchd agent (macOS).
    Launchd,
    /// A scheduled task started at logon (Windows).
    WindowsTask,
}

impl ServiceKind {
    /// The kind for an operating system name (`std::env::consts::OS`).
    #[must_use]
    pub fn for_os(os: &str) -> Option<Self> {
        match os {
            "linux" => Some(Self::Systemd),
            "macos" => Some(Self::Launchd),
            "windows" => Some(Self::WindowsTask),
            _ => None,
        }
    }
}

/// A file to write and the commands to run to install or remove the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePlan {
    /// The definition file and its content, if the service has one.
    pub file: Option<(PathBuf, String)>,
    /// Commands to run after writing the file (program and arguments).
    pub commands: Vec<Vec<String>>,
}

fn quote_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// What `tokred proxy service install` does.
#[must_use]
pub fn install_plan(kind: ServiceKind, exe: &Path, home: &Path) -> ServicePlan {
    let exe_text = exe.display().to_string();
    match kind {
        ServiceKind::Systemd => {
            let unit = format!(
                "[Unit]\nDescription=tokred local proxy for Claude Code\n\n[Service]\n\
                 ExecStart=\"{exe_text}\" proxy start --foreground\nRestart=on-failure\n\n\
                 [Install]\nWantedBy=default.target\n"
            );
            ServicePlan {
                file: Some((
                    home.join(format!(".config/systemd/user/{SERVICE_NAME}.service")),
                    unit,
                )),
                commands: vec![
                    owned(&["systemctl", "--user", "daemon-reload"]),
                    owned(&["systemctl", "--user", "enable", "--now", SERVICE_NAME]),
                ],
            }
        }
        ServiceKind::Launchd => {
            let path = home.join(format!(
                "Library/LaunchAgents/com.tokred.{SERVICE_NAME}.plist"
            ));
            let plist = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
                 \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                 <plist version=\"1.0\">\n<dict>\n\
                 <key>Label</key><string>com.tokred.{SERVICE_NAME}</string>\n\
                 <key>ProgramArguments</key><array>\
                 <string>{}</string><string>proxy</string><string>start</string>\
                 <string>--foreground</string></array>\n\
                 <key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n\
                 </dict>\n</plist>\n",
                quote_xml(&exe_text)
            );
            let path_text = path.display().to_string();
            ServicePlan {
                file: Some((path, plist)),
                commands: vec![owned(&["launchctl", "load", "-w", &path_text])],
            }
        }
        ServiceKind::WindowsTask => ServicePlan {
            file: None,
            commands: vec![owned(&[
                "schtasks",
                "/Create",
                "/F",
                "/SC",
                "ONLOGON",
                "/TN",
                SERVICE_NAME,
                "/TR",
                &format!("\"{exe_text}\" proxy start --foreground"),
            ])],
        },
    }
}

/// What `tokred proxy service uninstall` does: the commands to run, then the file to delete.
#[must_use]
pub fn uninstall_plan(kind: ServiceKind, home: &Path) -> ServicePlan {
    match kind {
        ServiceKind::Systemd => ServicePlan {
            file: Some((
                home.join(format!(".config/systemd/user/{SERVICE_NAME}.service")),
                String::new(),
            )),
            commands: vec![owned(&[
                "systemctl",
                "--user",
                "disable",
                "--now",
                SERVICE_NAME,
            ])],
        },
        ServiceKind::Launchd => {
            let path = home.join(format!(
                "Library/LaunchAgents/com.tokred.{SERVICE_NAME}.plist"
            ));
            let path_text = path.display().to_string();
            ServicePlan {
                file: Some((path, String::new())),
                commands: vec![owned(&["launchctl", "unload", "-w", &path_text])],
            }
        }
        ServiceKind::WindowsTask => ServicePlan {
            file: None,
            commands: vec![owned(&["schtasks", "/Delete", "/F", "/TN", SERVICE_NAME])],
        },
    }
}

fn owned(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

/// Runs one planned command, failing with its output when it fails.
pub fn run_planned(command: &[String]) -> Result<()> {
    let Some((program, args)) = command.split_first() else {
        return Ok(());
    };
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("could not run {program}"))?;
    if !output.status.success() {
        bail!(
            "`{}` failed: {}",
            command.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn the_secret_is_created_once_private_and_long() {
        let tmp = tempfile::tempdir().unwrap();
        let files = ProxyFiles::new(tmp.path().join("proxy"));
        let first = files.secret().unwrap();
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(files.secret().unwrap(), first, "reused");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(files.secret_path())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // A truncated secret is replaced.
        std::fs::write(files.secret_path(), "short").unwrap();
        assert_ne!(files.secret().unwrap(), "short");
        // Two fresh secrets differ.
        assert_ne!(random_hex(), random_hex());
    }

    #[test]
    fn an_unwritable_folder_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a file");
        std::fs::write(&file, "x").unwrap();
        let files = ProxyFiles::new(file.join("proxy"));
        assert!(files.secret().is_err());
        assert!(files.write_pid(1, 2).is_err());
    }

    #[test]
    fn pid_files_round_trip_and_reject_garbage() {
        let tmp = tempfile::tempdir().unwrap();
        let files = ProxyFiles::new(tmp.path().join("proxy"));
        assert_eq!(files.read_pid(), None);
        files.write_pid(4242, 8787).unwrap();
        assert_eq!(files.read_pid(), Some((4242, 8787)));
        files.clear_pid();
        assert_eq!(files.read_pid(), None);
        files.clear_pid();
        for bad in ["", "12", "x 8787", "12 port", "12 99999"] {
            assert_eq!(parse_pid(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn urls_and_ports() {
        assert_eq!(base_url(8787), "http://127.0.0.1:8787");
        assert_eq!(port_of("127.0.0.1:8787").unwrap(), 8787);
        assert_eq!(port_of("[::1]:9000").unwrap(), 9000);
        assert!(port_of("localhost").is_err());
        assert!(port_of("x:y").is_err());
    }

    /// Answers one request with `body`.
    fn serve_once(body: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0_u8; 1024];
                let _ = stream.read(&mut buffer);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        port
    }

    #[test]
    fn health_checks_need_the_tokred_answer() {
        assert!(healthy(
            serve_once(r#"{"ok":true,"version":"0.1.0"}"#),
            Duration::from_secs(2)
        ));
        assert!(!healthy(
            serve_once(r#"{"something":"else"}"#),
            Duration::from_secs(2)
        ));
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(!healthy(closed, Duration::from_millis(300)));
    }

    #[test]
    fn waiting_stops_on_success_or_timeout() {
        let mut calls = 0;
        assert!(wait_until(
            Duration::from_secs(1),
            Duration::from_millis(1),
            &mut || {
                calls += 1;
                calls == 3
            }
        ));
        assert_eq!(calls, 3);
        assert!(!wait_until(
            Duration::from_millis(20),
            Duration::from_millis(5),
            &mut || false
        ));
    }

    #[test]
    fn detached_processes_start_and_missing_programs_fail() {
        let me = std::env::current_exe().unwrap();
        // The test binary with a filter that matches nothing exits quickly.
        let pid = spawn_detached(&me, &["--list", "--exact", "no_such_test"]).unwrap();
        assert!(pid > 0);
        assert!(spawn_detached(Path::new("/no/such/tokred"), &[]).is_err());
    }

    #[test]
    fn service_kinds_per_os() {
        assert_eq!(ServiceKind::for_os("linux"), Some(ServiceKind::Systemd));
        assert_eq!(ServiceKind::for_os("macos"), Some(ServiceKind::Launchd));
        assert_eq!(
            ServiceKind::for_os("windows"),
            Some(ServiceKind::WindowsTask)
        );
        assert_eq!(ServiceKind::for_os("freebsd"), None);
    }

    #[test]
    fn install_and_uninstall_plans() {
        let home = Path::new("/home/u");
        let exe = Path::new("/opt/bin/tok&red");

        let systemd = install_plan(ServiceKind::Systemd, exe, home);
        let (path, unit) = systemd.file.unwrap();
        assert_eq!(path, home.join(".config/systemd/user/tokred-proxy.service"));
        assert!(unit.contains("ExecStart=\"/opt/bin/tok&red\" proxy start --foreground"));
        assert_eq!(
            systemd.commands[1],
            owned(&["systemctl", "--user", "enable", "--now", "tokred-proxy"])
        );

        let launchd = install_plan(ServiceKind::Launchd, exe, home);
        let (path, plist) = launchd.file.unwrap();
        assert!(path.ends_with("Library/LaunchAgents/com.tokred.tokred-proxy.plist"));
        assert!(
            plist.contains("<string>/opt/bin/tok&amp;red</string>"),
            "escaped"
        );
        assert!(plist.contains("<key>RunAtLoad</key><true/>"));
        assert_eq!(
            launchd.commands[0][..3],
            owned(&["launchctl", "load", "-w"])[..]
        );

        let task = install_plan(ServiceKind::WindowsTask, exe, home);
        assert!(task.file.is_none());
        assert!(task.commands[0].contains(&"ONLOGON".to_owned()));

        for kind in [
            ServiceKind::Systemd,
            ServiceKind::Launchd,
            ServiceKind::WindowsTask,
        ] {
            let plan = uninstall_plan(kind, home);
            assert_eq!(plan.commands.len(), 1);
            assert_eq!(plan.file.is_some(), kind != ServiceKind::WindowsTask);
        }
    }

    #[test]
    fn planned_commands_report_failures() {
        run_planned(&[]).unwrap();
        let me = std::env::current_exe().unwrap().display().to_string();
        run_planned(&[me.clone(), "--list".to_owned()]).unwrap();
        let err = run_planned(&[me, "--definitely-not-a-flag".to_owned()]).unwrap_err();
        assert!(err.to_string().contains("failed"), "{err}");
        assert!(run_planned(&["/no/such/program".to_owned()]).is_err());
    }
}
