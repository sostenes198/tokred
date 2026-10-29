//! End to end: `tokred initialize`, `proxy enable|disable`, the `session-start` safety net and
//! `tokred claude` with the proxy switched off, through the real binary in a sandbox.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Stdio;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

struct Sandbox {
    root: tempfile::TempDir,
    port: u16,
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

impl Sandbox {
    /// A configured installation whose proxy listens on a free port, enabled or not.
    fn new(enabled: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("my claude/projects")).unwrap();
        std::fs::create_dir_all(root.path().join("home")).unwrap();
        std::fs::write(root.path().join("my claude/settings.json"), "{}").unwrap();
        let sb = Self {
            root,
            port: free_port(),
        };
        sb.tokred().args(["configure", "--yes"]).assert().success();
        let path = sb.data().join("tokred.toml");
        let toml = std::fs::read_to_string(&path).unwrap();
        assert!(
            toml.contains("enabled = false"),
            "configure leaves the proxy off: {toml}"
        );
        let toml = toml
            .replace("enabled = false", &format!("enabled = {enabled}"))
            .replace(
                "bind = \"127.0.0.1:1029\"",
                &format!("bind = \"127.0.0.1:{}\"", sb.port),
            )
            // No request is ever sent, but nothing may point at the real API.
            .replace(
                "upstream = \"https://api.anthropic.com\"",
                &format!("upstream = \"http://127.0.0.1:{}\"", free_port()),
            );
        std::fs::write(path, toml).unwrap();
        sb
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("tokred data")
    }

    fn settings(&self) -> PathBuf {
        self.root.path().join("my claude/settings.json")
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn tokred(&self) -> Command {
        let home = self.root.path().join("home");
        let mut cmd = cargo_bin_cmd!("tokred");
        cmd.env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", home.join("appdata"))
            .env("XDG_DATA_HOME", home.join("xdg"))
            .env("CLAUDE_CONFIG_DIR", self.root.path().join("my claude"))
            .env("TOKRED_DATA_DIR", self.data())
            .env("TOKRED_OFFLINE", "1")
            .env_remove("ANTHROPIC_BASE_URL")
            .env_remove("NO_COLOR");
        cmd
    }

    #[cfg(unix)]
    /// Runs `tokred args...` with no standard streams captured (a command that may start the
    /// background proxy must not hand it the harness's pipes).
    fn run_plain(&self, args: &[&str]) -> std::process::ExitStatus {
        let command = self.tokred();
        let mut plain = std::process::Command::new(command.get_program());
        for (name, value) in command.get_envs() {
            match value {
                Some(value) => plain.env(name, value),
                None => plain.env_remove(name),
            };
        }
        plain
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
    }

    fn write_settings(&self, text: &str) {
        std::fs::write(self.settings(), text).unwrap();
    }

    fn settings_text(&self) -> String {
        std::fs::read_to_string(self.settings()).unwrap()
    }

    /// Whether a tokred proxy answers on the port.
    fn proxy_answers(&self) -> bool {
        answers_on(self.port)
    }

    #[cfg(unix)]
    fn wait_for_proxy(&self) -> bool {
        wait_for(self.port)
    }

    #[cfg(unix)]
    fn stop_proxy(&self) {
        self.tokred().args(["proxy", "stop"]).assert().success();
    }
}

/// Whether a tokred proxy answers on `port`.
fn answers_on(port: u16) -> bool {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let request = format!(
        "GET /__tokred/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nconnection: close\r\n\r\n"
    );
    let _ = stream.write_all(request.as_bytes());
    let mut text = String::new();
    let _ = stream.read_to_string(&mut text);
    text.contains("\"ok\":true")
}

/// Waits up to ten seconds for a tokred proxy to answer on `port`.
#[cfg(unix)]
fn wait_for(port: u16) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if answers_on(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn disabled_removes_tokreds_url_and_leaves_any_other_gateway() {
    let sb = Sandbox::new(false);
    sb.write_settings(&format!(
        r#"{{"model":"opus","env":{{"FOO":"1","ANTHROPIC_BASE_URL":"{}"}}}}"#,
        sb.url()
    ));
    sb.tokred()
        .arg("initialize")
        .assert()
        .success()
        .stdout("")
        .stderr("");
    let text = sb.settings_text();
    assert!(!text.contains("ANTHROPIC_BASE_URL"), "{text}");
    assert!(text.contains("\"FOO\"") && text.contains("opus"), "{text}");

    let gateway = r#"{"env":{"ANTHROPIC_BASE_URL":"https://gateway.example"}}"#;
    sb.write_settings(gateway);
    sb.tokred()
        .args(["initialize", "--verbose"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "left ANTHROPIC_BASE_URL=https://gateway.example",
        ));
    assert_eq!(sb.settings_text(), gateway, "untouched");

    sb.tokred()
        .args(["initialize", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"state\":\"disabled\""));
    assert!(!sb.proxy_answers(), "disabled starts nothing");
}

#[test]
fn invalid_arguments_are_the_only_failure() {
    let sb = Sandbox::new(true);
    sb.tokred()
        .args(["initialize", "--nope"])
        .assert()
        .failure()
        .code(2);
    sb.tokred()
        .args(["initialize", "--quiet", "--verbose"])
        .assert()
        .failure()
        .code(2);
    // Broken settings and a broken config are warnings, not failures.
    sb.write_settings("{ nope");
    sb.tokred()
        .arg("initialize")
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains(
            "could not read Claude Code's settings.json",
        ));
    std::fs::write(sb.data().join("tokred.toml"), "[[[").unwrap();
    sb.tokred().arg("initialize").assert().success();
}

#[test]
fn enabled_never_overwrites_another_gateway_and_starts_nothing() {
    let sb = Sandbox::new(true);
    let gateway = r#"{"env":{"ANTHROPIC_BASE_URL":"https://gateway.example"}}"#;
    sb.write_settings(gateway);
    sb.tokred()
        .arg("initialize")
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains(
            "points to https://gateway.example",
        ));
    assert_eq!(sb.settings_text(), gateway);
    assert!(!sb.proxy_answers());

    // The same for a gateway exported in the shell.
    sb.write_settings("{}");
    sb.tokred()
        .env("ANTHROPIC_BASE_URL", "https://other.example")
        .arg("initialize")
        .assert()
        .success()
        .stderr(predicate::str::contains("points to https://other.example"));
    assert_eq!(sb.settings_text(), "{}");
    assert!(!sb.proxy_answers());
}

#[cfg(unix)]
#[test]
fn enabled_starts_the_proxy_points_claude_at_it_and_is_quick_the_second_time() {
    let sb = Sandbox::new(true);
    sb.write_settings(r#"{"model":"opus"}"#);

    // The first launch has to start the proxy; no output unless there is a problem.
    assert!(sb.run_plain(&["initialize"]).success());
    assert!(sb.proxy_answers(), "initialize started the proxy");
    let text = sb.settings_text();
    assert!(text.contains(&sb.url()) && text.contains("opus"), "{text}");

    // Every later launch only reads the file and probes the proxy.
    let started = Instant::now();
    sb.tokred()
        .arg("initialize")
        .assert()
        .success()
        .stdout("")
        .stderr("");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(sb.settings_text(), text, "no rewrite when nothing changed");

    // `proxy disable` removes the URL at once and leaves the running proxy alone.
    sb.tokred()
        .args(["proxy", "disable"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[proxy] enabled = false"));
    assert!(!sb.settings_text().contains("ANTHROPIC_BASE_URL"));
    assert!(sb.proxy_answers());
    sb.tokred()
        .args(["proxy", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tokred proxy: running"));

    // ... and `proxy enable` sets it again; `unconfigure` takes tokred's URL away.
    sb.tokred()
        .args(["proxy", "enable"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[proxy] enabled = true"));
    assert!(sb.settings_text().contains(&sb.url()));
    sb.tokred()
        .arg("unconfigure")
        .assert()
        .success()
        .stdout(predicate::str::contains("ANTHROPIC_BASE_URL removed"));
    assert!(!sb.settings_text().contains("ANTHROPIC_BASE_URL"));
    sb.stop_proxy();
}

#[cfg(unix)]
#[test]
fn a_proxy_that_cannot_start_does_not_leave_a_dead_url_behind() {
    let sb = Sandbox::new(true);
    // Something that is not tokred holds the port, so the proxy cannot bind it.
    let squatter = TcpListener::bind(("127.0.0.1", sb.port)).unwrap();
    std::thread::spawn(move || {
        for stream in squatter.incoming() {
            drop(stream);
        }
    });
    sb.write_settings(&format!(
        r#"{{"env":{{"ANTHROPIC_BASE_URL":"{}"}}}}"#,
        sb.url()
    ));
    sb.tokred()
        .arg("initialize")
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains("could not be started"));
    let text = sb.settings_text();
    assert!(!text.contains("ANTHROPIC_BASE_URL"), "{text}");
}

#[cfg(unix)]
#[test]
fn session_start_brings_a_dead_proxy_back_only_when_it_is_enabled_and_in_use() {
    let payload =
        r#"{"session_id":"s1","hook_event_name":"SessionStart","source":"startup","cwd":"/tmp"}"#;

    // Enabled, and settings.json points Claude Code at the proxy that is not running.
    let sb = Sandbox::new(true);
    sb.write_settings(&format!(
        r#"{{"env":{{"ANTHROPIC_BASE_URL":"{}"}}}}"#,
        sb.url()
    ));
    sb.tokred()
        .args(["hook", "session-start"])
        .write_stdin(payload)
        .assert()
        .success()
        .stdout("");
    assert!(sb.wait_for_proxy(), "the hook started the proxy");
    sb.stop_proxy();

    // Enabled but not in use (no URL anywhere): nothing is started.
    sb.write_settings("{}");
    sb.tokred()
        .args(["hook", "session-start"])
        .write_stdin(payload)
        .assert()
        .success();
    std::thread::sleep(Duration::from_millis(500));
    assert!(!sb.proxy_answers());

    // In use through the session's environment instead of the file.
    sb.tokred()
        .env("ANTHROPIC_BASE_URL", sb.url())
        .args(["hook", "session-start"])
        .write_stdin(payload)
        .assert()
        .success();
    assert!(sb.wait_for_proxy());
    sb.stop_proxy();

    // Disabled: never.
    let off = Sandbox::new(false);
    off.write_settings(&format!(
        r#"{{"env":{{"ANTHROPIC_BASE_URL":"{}"}}}}"#,
        off.url()
    ));
    off.tokred()
        .args(["hook", "session-start"])
        .write_stdin(payload)
        .assert()
        .success();
    std::thread::sleep(Duration::from_millis(500));
    assert!(!off.proxy_answers());
}

#[cfg(unix)]
#[test]
fn tokred_claude_runs_claude_directly_while_the_proxy_is_disabled() {
    let sb = Sandbox::new(false);
    let bin = sb.root.path().join("fakebin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("claude");
    std::fs::write(
        &script,
        "#!/bin/sh\necho \"base=${ANTHROPIC_BASE_URL:-none} args=$*\"\nexit 4\n",
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    sb.tokred()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .args(["claude", "-p", "hi"])
        .assert()
        .code(4)
        .stdout(predicate::str::contains("base=none args=-p hi"))
        .stderr(predicate::str::contains("proxy is disabled"));
    assert!(!sb.proxy_answers());
}

#[test]
fn doctor_shows_the_switch() {
    let sb = Sandbox::new(false);
    sb.tokred()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("proxy:       disabled"));
}

/// `proxy enable` saves its answers, starts the proxy and, asked for another port, moves it.
#[cfg(unix)]
#[test]
fn proxy_enable_saves_its_answers_and_moves_a_running_proxy_to_the_new_port() {
    let sb = Sandbox::new(false);
    let first = sb.port.to_string();
    let status = sb.run_plain(&[
        "proxy",
        "enable",
        "--port",
        &first,
        "--record-content",
        "Yes",
    ]);
    assert!(status.success());
    assert!(sb.wait_for_proxy());
    let toml = std::fs::read_to_string(sb.data().join("tokred.toml")).unwrap();
    assert!(toml.contains("enabled = true"), "{toml}");
    assert!(toml.contains("log_bodies = true"), "{toml}");
    assert!(
        toml.contains(&format!("bind = \"127.0.0.1:{first}\"")),
        "{toml}"
    );
    assert!(sb.settings_text().contains(&sb.url()));

    let other = free_port();
    let status = sb.run_plain(&["proxy", "enable", "--port", &other.to_string()]);
    assert!(status.success());
    assert!(wait_for(other), "the proxy runs on the new port");
    assert!(!sb.proxy_answers(), "and no longer on the old one");
    let toml = std::fs::read_to_string(sb.data().join("tokred.toml")).unwrap();
    assert!(
        toml.contains(&format!("bind = \"127.0.0.1:{other}\"")),
        "{toml}"
    );
    assert!(
        toml.contains("log_bodies = true"),
        "the saved answer stays: {toml}"
    );
    assert!(
        sb.settings_text()
            .contains(&format!("http://127.0.0.1:{other}"))
    );
    sb.stop_proxy();
}
