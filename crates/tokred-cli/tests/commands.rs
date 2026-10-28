//! End-to-end tests of the commands that talk to the network, against a tiny local HTTP server.
//!
//! Nothing here reaches the internet or the real `~/.claude`: the sources are redirected with
//! `TOKRED_PRICES_URL` / `TOKRED_RELEASES_URL` and everything lives in a temporary directory.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("my claude/projects")).unwrap();
        std::fs::create_dir_all(root.path().join("home")).unwrap();
        std::fs::write(root.path().join("my claude/settings.json"), "{}").unwrap();
        Self { root }
    }

    fn claude(&self) -> PathBuf {
        self.root.path().join("my claude")
    }

    /// The binary, sandboxed and offline unless a test says otherwise.
    fn tokred(&self) -> Command {
        let home = self.root.path().join("home");
        let mut cmd = cargo_bin_cmd!("tokred");
        cmd.env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("APPDATA", home.join("appdata"))
            .env("XDG_DATA_HOME", home.join("xdg"))
            .env("CLAUDE_CONFIG_DIR", self.claude())
            .env("TOKRED_DATA_DIR", self.root.path().join("tokred data"))
            .env("TOKRED_OFFLINE", "1")
            .env_remove("TOKRED_NO_UPDATE_CHECK")
            .env_remove("TOKRED_LOG")
            .env_remove("TOKRED_PRICES_URL")
            .env_remove("TOKRED_RELEASES_URL");
        cmd
    }

    fn configure(&self) {
        self.tokred()
            .args(["configure", "--yes"])
            .assert()
            .success();
    }
}

/// Serves fixed answers by path on a free local port; returns the base URL.
fn fake_server(routes: Vec<(&'static str, u16, String)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0_u8; 4096];
            let read = stream.read(&mut buffer).unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (status, body) = routes
                .iter()
                .find(|(route, _, _)| *route == path)
                .map_or((404, String::from("not found")), |(_, status, body)| {
                    (*status, body.clone())
                });
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    base
}

const PRICES: &str = r#"{"claude-test-20990101":{"litellm_provider":"anthropic","mode":"chat","input_cost_per_token":0.000003,"output_cost_per_token":0.000015,"cache_creation_input_token_cost":0.00000375,"cache_read_input_token_cost":3e-7}}"#;

fn release(version: &str) -> String {
    format!(
        r#"{{"tag_name":"v{version}","html_url":"https://example.invalid/releases/v{version}","draft":false}}"#
    )
}

// ---- audit ----

#[test]
fn audit_lists_the_fixed_context_and_unused_mcp_servers() {
    let sb = Sandbox::new();
    std::fs::write(sb.claude().join("CLAUDE.md"), "x".repeat(8_000)).unwrap();
    let project = sb.root.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join(".mcp.json"),
        r#"{"mcpServers":{"idle-server":{"command":"x"}}}"#,
    )
    .unwrap();

    sb.tokred()
        .args(["audit", "--project"])
        .arg(&project)
        .assert()
        .success()
        .stdout(predicate::str::contains("tokred Context Audit"))
        .stdout(predicate::str::contains("Heaviest Items"))
        .stdout(predicate::str::contains("idle-server"));

    let out = sb
        .tokred()
        .args(["audit", "--json", "--project"])
        .arg(&project)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(json["summary"]["total_estimated_tokens"].as_u64().unwrap() >= 2_000);
}

// ---- pricing ----

#[test]
fn pricing_show_uses_the_embedded_snapshot_and_sync_refreshes_it() {
    let sb = Sandbox::new();
    sb.configure();

    sb.tokred()
        .args(["pricing", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "tokred Prices (USD per million tokens)",
        ))
        .stdout(predicate::str::contains("claude-"));

    let base = fake_server(vec![("/prices", 200, PRICES.to_owned())]);
    sb.tokred()
        .env_remove("TOKRED_OFFLINE")
        .env("TOKRED_PRICES_URL", format!("{base}/prices"))
        .args(["pricing", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Synced the prices of 1 models."));
    sb.tokred()
        .args(["pricing", "show", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-test-20990101"));
}

#[test]
fn pricing_sync_reports_an_unreachable_source() {
    let sb = Sandbox::new();
    sb.configure();
    let base = fake_server(vec![("/prices", 500, "boom".to_owned())]);
    sb.tokred()
        .env_remove("TOKRED_OFFLINE")
        .env("TOKRED_PRICES_URL", format!("{base}/prices"))
        .args(["pricing", "sync"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("could not refresh the prices"));
}

// ---- update ----

#[test]
fn update_needs_the_network() {
    let sb = Sandbox::new();
    sb.tokred()
        .arg("update")
        .assert()
        .failure()
        .stderr(predicate::str::contains("TOKRED_OFFLINE"));
}

#[test]
fn update_reports_up_to_date_and_unreachable() {
    let sb = Sandbox::new();
    let version = env!("CARGO_PKG_VERSION");
    let base = fake_server(vec![
        ("/ok", 200, release(version)),
        ("/down", 503, "no".to_owned()),
    ]);

    sb.tokred()
        .env_remove("TOKRED_OFFLINE")
        .env("TOKRED_RELEASES_URL", format!("{base}/ok"))
        .arg("update")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "tokred {version} is up to date."
        )));

    sb.tokred()
        .env_remove("TOKRED_OFFLINE")
        .env("TOKRED_RELEASES_URL", format!("{base}/down"))
        .arg("update")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "could not check for the latest release",
        ));
}

// ---- background chores ----

#[test]
fn a_new_version_is_announced_on_stderr_and_never_pollutes_json() {
    let sb = Sandbox::new();
    sb.configure();
    let base = fake_server(vec![
        ("/release", 200, release("99.0.0")),
        ("/prices", 200, PRICES.to_owned()),
    ]);

    let output = sb
        .tokred()
        .env_remove("TOKRED_OFFLINE")
        .env("TOKRED_RELEASES_URL", format!("{base}/release"))
        .env("TOKRED_PRICES_URL", format!("{base}/prices"))
        .args(["doctor", "--json"])
        .assert()
        .success()
        .stderr(predicate::str::contains("new version 99.0.0 available"))
        .get_output()
        .clone();
    // stdout is still one clean JSON document.
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();

    // The daily price sync ran too, so the synced model is now in the table.
    sb.tokred()
        .args(["pricing", "show", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-test-20990101"));
}

#[test]
fn the_version_check_can_be_switched_off() {
    let sb = Sandbox::new();
    sb.configure();
    let base = fake_server(vec![("/release", 200, release("99.0.0"))]);
    sb.tokred()
        .env_remove("TOKRED_OFFLINE")
        .env("TOKRED_NO_UPDATE_CHECK", "1")
        .env("TOKRED_RELEASES_URL", format!("{base}/release"))
        .env("TOKRED_PRICES_URL", format!("{base}/none"))
        .arg("doctor")
        .assert()
        .success()
        .stderr(predicate::str::contains("new version").not());
}
