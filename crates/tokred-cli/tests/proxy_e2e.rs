//! End to end: the real `tokred` binary runs the proxy in the background against a fake Anthropic
//! server; requests go through it and what it stored is read back with the CLI.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines
)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;

const SSE: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"claude-sonnet-5-5\",\"usage\":{\"input_tokens\":10,\"cache_creation_input_tokens\":100,\"cache_read_input_tokens\":0,\"output_tokens\":1,\"cache_creation\":{\"ephemeral_5m_input_tokens\":0,\"ephemeral_1h_input_tokens\":100}}}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":25}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

struct Sandbox {
    root: tempfile::TempDir,
}

impl Sandbox {
    fn new(upstream: &str, port: u16) -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("my claude/projects")).unwrap();
        std::fs::create_dir_all(root.path().join("home")).unwrap();
        std::fs::write(root.path().join("my claude/settings.json"), "{}").unwrap();
        let sb = Self { root };
        sb.tokred().args(["configure", "--yes"]).assert().success();
        let toml = std::fs::read_to_string(sb.data().join("tokred.toml")).unwrap();
        let toml = toml
            .replace(
                "bind = \"127.0.0.1:8787\"",
                &format!("bind = \"127.0.0.1:{port}\""),
            )
            .replace(
                "upstream = \"https://api.anthropic.com\"",
                &format!("upstream = \"{upstream}\""),
            );
        std::fs::write(sb.data().join("tokred.toml"), toml).unwrap();
        sb
    }

    fn data(&self) -> PathBuf {
        self.root.path().join("tokred data")
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
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// A fake Anthropic API: answers `/v1/messages` with a streamed response and remembers the
/// bodies and headers it was sent.
/// What the fake upstream saw: each request's head and body.
type Seen = Arc<Mutex<Vec<(String, String)>>>;

fn fake_upstream() -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let log = Arc::clone(&log);
            std::thread::spawn(move || {
                let mut data = Vec::new();
                let mut buffer = [0_u8; 8192];
                loop {
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    data.extend_from_slice(&buffer[..read]);
                    let text = String::from_utf8_lossy(&data).into_owned();
                    if let Some(split) = text.find("\r\n\r\n") {
                        let length = text[..split]
                            .to_lowercase()
                            .lines()
                            .find_map(|l| {
                                l.strip_prefix("content-length: ")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if data.len() >= split + 4 + length {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&data).into_owned();
                let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
                log.lock().unwrap().push((head.to_owned(), body.to_owned()));
                // Like the real API: compress the stream when the client accepts gzip.
                let gzip = head
                    .to_lowercase()
                    .lines()
                    .any(|l| l.starts_with("accept-encoding:") && l.contains("gzip"));
                let (encoding, payload) = if gzip {
                    let mut encoder =
                        flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                    encoder.write_all(SSE.as_bytes()).unwrap();
                    ("content-encoding: gzip\r\n", encoder.finish().unwrap())
                } else {
                    ("", SSE.as_bytes().to_vec())
                };
                let response_head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n{encoding}content-length: {}\r\nrequest-id: req_test\r\nconnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(response_head.as_bytes());
                let _ = stream.write_all(&payload);
            });
        }
    });
    (url, seen)
}

fn post(port: u16, headers: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let request = format!(
        "POST /v1/messages?beta=true HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{headers}connection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn the_real_proxy_forwards_streams_records_and_stops() {
    let (upstream, seen) = fake_upstream();
    let port = free_port();
    let sb = Sandbox::new(&upstream, port);

    sb.tokred()
        .args(["proxy", "start"])
        .assert()
        .success()
        .stdout(predicate::str::contains("The tokred proxy is running"));
    sb.tokred()
        .args(["proxy", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tokred proxy: running"));

    // A main-loop request (tools, thinking) and a subagent request (agent-id header).
    let main_body = r#"{"model":"claude-sonnet-5-5","stream":true,"thinking":{"type":"adaptive"},"tools":[{"name":"Read"}],"messages":[{"role":"user","content":"hello there"}]}"#;
    let reply = post(
        port,
        "x-api-key: sk-test-secret\r\nx-claude-code-session-id: sess-1\r\nanthropic-beta: effort-2025-11-24\r\naccept-encoding: gzip, deflate\r\n",
        main_body,
    );
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(
        reply.contains("message_stop"),
        "the stream passed through: {reply}"
    );
    let sub_body = r#"{"model":"claude-sonnet-5-5","stream":true,"tools":[{"name":"Read"},{"name":"Grep"}],"messages":[{"role":"user","content":"find the config file"}]}"#;
    post(
        port,
        "x-api-key: sk-test-secret\r\nx-claude-code-session-id: sess-1\r\nx-claude-code-agent-id: agent-9\r\n",
        sub_body,
    );

    // The upstream saw the exact bodies and the unchanged credential and beta headers.
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].1, main_body, "byte for byte");
        let head = seen[0].0.to_lowercase();
        assert!(head.contains("x-api-key: sk-test-secret"));
        assert!(head.contains("anthropic-beta: effort-2025-11-24"));
        assert!(!head.contains("x-tokred-secret"));
    }

    let pid: u32 = std::fs::read_to_string(sb.data().join("proxy/pid"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    sb.tokred()
        .args(["proxy", "stop"])
        .assert()
        .success()
        .stdout(predicate::str::contains("stopped"));
    sb.tokred()
        .args(["proxy", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not running"));
    // The process really ends (it used to linger when the record writer was never released).
    #[cfg(unix)]
    {
        let mut gone = false;
        for _ in 0..50 {
            let alive = std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .unwrap()
                .success();
            if !alive {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(
            gone,
            "the proxy process {pid} is still running after `proxy stop`"
        );
    }
    #[cfg(not(unix))]
    let _ = pid;

    // Stopping flushed the writer: both requests are in the database, with the 1-hour cache split.
    let db = rusqlite::Connection::open(sb.data().join("tokred.db")).unwrap();
    let rows: Vec<(String, String, i64, i64, i64)> = db
        .prepare("SELECT origin, model_effective, cache_write, cache_write_1h, output FROM proxy_requests ORDER BY ts, id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    let origins: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert!(
        origins.contains(&"main") && origins.contains(&"subagent"),
        "{origins:?}"
    );
    assert!(
        rows.iter()
            .all(|r| r.1 == "claude-sonnet-5-5" && r.2 == 100 && r.3 == 100 && r.4 == 25)
    );
    // Credentials never reach the database.
    let dump: String = db
        .prepare(
            "SELECT group_concat(id || ts || session_id || tools_hash, '') FROM proxy_requests",
        )
        .unwrap()
        .query_row([], |r| r.get::<_, Option<String>>(0))
        .unwrap()
        .unwrap_or_default();
    assert!(!dump.contains("sk-test-secret"));
}

#[test]
fn a_request_to_a_non_loopback_host_name_is_refused() {
    let (upstream, _) = fake_upstream();
    let port = free_port();
    let sb = Sandbox::new(&upstream, port);
    sb.tokred().args(["proxy", "start"]).assert().success();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .write_all(b"POST /v1/messages HTTP/1.1\r\nHost: evil.example\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}")
        .unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).unwrap();
    assert!(out.starts_with("HTTP/1.1 421"), "{out}");
    sb.tokred().args(["proxy", "stop"]).assert().success();
}

#[test]
fn tokred_claude_falls_back_to_plain_claude_when_the_proxy_cannot_start() {
    let (upstream, _) = fake_upstream();
    let sb = Sandbox::new(&upstream, free_port());
    // A fake `claude` that prints the base URL it was started with.
    let bin = sb.root.path().join("fakebin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("claude");
    std::fs::write(
        &script,
        "#!/bin/sh\necho \"base=${ANTHROPIC_BASE_URL:-none} args=$*\"\nexit 3\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // The other gateway wins, and claude's exit code is tokred's exit code.
    #[cfg(unix)]
    sb.tokred()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("ANTHROPIC_BASE_URL", "https://gateway.example")
        .args(["claude", "-p", "hi"])
        .assert()
        .code(3)
        .stdout(predicate::str::contains(
            "base=https://gateway.example args=-p hi",
        ))
        .stderr(predicate::str::contains(
            "already set to https://gateway.example",
        ));
}

#[test]
fn tokred_claude_starts_the_proxy_and_points_claude_at_it() {
    let (upstream, _) = fake_upstream();
    let port = free_port();
    let sb = Sandbox::new(&upstream, port);
    let bin = sb.root.path().join("fakebin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("claude");
    std::fs::write(&script, "#!/bin/sh\necho \"base=$ANTHROPIC_BASE_URL\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Nothing runs yet: `tokred claude` has to start the proxy first, then run claude.
        sb.tokred()
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .arg("claude")
            .assert()
            .success()
            .stdout(predicate::str::contains(format!(
                "base=http://127.0.0.1:{port}"
            )));
        sb.tokred()
            .args(["proxy", "status"])
            .assert()
            .success()
            .stdout(predicate::str::contains("tokred proxy: running"));
        sb.tokred().args(["proxy", "stop"]).assert().success();
    }
    #[cfg(not(unix))]
    let _ = (port, script);
}

#[test]
fn the_advice_commands_work_through_the_real_binary() {
    let (upstream, _) = fake_upstream();
    let sb = Sandbox::new(&upstream, free_port());

    sb.tokred()
        .args(["routing", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tokred Model Routing"));
    sb.tokred()
        .args(["routing", "on"])
        .assert()
        .success()
        .stdout(predicate::str::contains("enforce"));
    sb.tokred()
        .args(["routing", "off"])
        .assert()
        .success()
        .stdout(predicate::str::contains("observe"));
    sb.tokred()
        .args(["routing", "explain", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no routing decision"));
    sb.tokred()
        .arg("suggest")
        .assert()
        .success()
        .stdout(predicate::str::contains("tokred Model Suggestions"));
    sb.tokred()
        .arg("statusline")
        .write_stdin(r#"{"session_id":"s1","model":{"id":"claude-opus-5-5"}}"#)
        .assert()
        .success();
    sb.tokred()
        .args(["compression", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("compression mode:   observe"));
    sb.tokred()
        .args(["compression", "on"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Registered the `tokred_retrieve` MCP server",
        ));
    sb.tokred()
        .args(["compression", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("registered in Claude Code"));
    sb.tokred().args(["compression", "off"]).assert().success();

    // The MCP server speaks JSON-RPC on stdio and answers `tools/list`.
    let output = sb
        .tokred()
        .arg("mcp")
        .write_stdin(concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#, "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#, "\n",
        ))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("tokred_retrieve"), "{text}");
}
