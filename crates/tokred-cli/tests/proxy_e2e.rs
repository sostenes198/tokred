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
                "bind = \"127.0.0.1:1029\"",
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

    /// Sets a key of the `[proxy]` section in `tokred.toml`.
    fn set_proxy_key(&self, key: &str, value: &str) {
        let path = self.data().join("tokred.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        let line = format!("{key} = {value}");
        let mut found = false;
        let mut lines: Vec<String> = text
            .lines()
            .map(|l| {
                if l.trim_start().starts_with(&format!("{key} =")) {
                    found = true;
                    line.clone()
                } else {
                    l.to_owned()
                }
            })
            .collect();
        if !found {
            let at = lines.iter().position(|l| l.trim() == "[proxy]").unwrap();
            lines.insert(at + 1, line);
        }
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
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

impl Sandbox {
    /// Runs `tokred <args>` with no standard streams attached (nothing to inherit) and returns
    /// whether it succeeded.
    fn run_detached(&self, args: &[&str]) -> bool {
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
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    /// Starts the background proxy and waits until it answers.
    ///
    /// The output is not captured: on Windows the detached proxy would inherit the pipes a
    /// capturing harness gives its child and keep them open, so the harness would wait for the
    /// proxy to exit (the test below covers the captured case on its own).
    fn start_proxy(&self) {
        assert!(
            self.run_detached(&["proxy", "start"]),
            "`proxy start` failed"
        );
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

    sb.start_proxy();
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
    // The richer per-request fields: stop reason, tool names, routing mode in force.
    let extra: Vec<(String, String, Option<String>, String)> = db
        .prepare("SELECT stop_reason, tools, cache_ttl, routing_mode FROM proxy_requests WHERE origin = 'main'")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        extra,
        [(
            "end_turn".to_owned(),
            r#"["Read"]"#.to_owned(),
            None,
            "observe".to_owned()
        )]
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
    sb.start_proxy();
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

#[test]
fn status_and_doctor_show_the_corporate_network_settings_without_credentials() {
    let sb = Sandbox::new("https://api.anthropic.com", free_port());
    let with_network = |args: &[&str]| {
        let output = sb
            .tokred()
            .args(args)
            .env("HTTPS_PROXY", "http://alice:s3cret@proxy.corp:3128")
            .env("NODE_EXTRA_CA_CERTS", "/no/such/corp-ca.pem")
            .env_remove("NO_PROXY")
            .env_remove("no_proxy")
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        String::from_utf8(output).unwrap()
    };

    let status = with_network(&["proxy", "status", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&status).unwrap();
    assert_eq!(value["network"]["proxy"]["host"], "proxy.corp:3128");
    assert_eq!(value["network"]["proxy"]["status"], "tunnel");
    assert_eq!(value["network"]["ca_files"][0]["readable"], false);

    let doctor = with_network(&["doctor"]);
    assert!(
        doctor.contains("network: HTTPS_PROXY proxy.corp:3128"),
        "{doctor}"
    );
    assert!(
        doctor.contains("extra CA file /no/such/corp-ca.pem"),
        "{doctor}"
    );
    for text in [&status, &doctor] {
        assert!(
            !text.contains("s3cret") && !text.contains("alice"),
            "{text}"
        );
    }
}

/// Stops the proxy of a sandbox when the test ends, even when it fails.
struct StopProxy<'a>(&'a Sandbox);

impl Drop for StopProxy<'_> {
    fn drop(&mut self) {
        self.0.run_detached(&["proxy", "stop"]);
    }
}

/// Regression: the background proxy must not keep the pipes of a caller that captures the output
/// of `proxy start` open (on Windows a detached child inherits them, and `$(tokred proxy start)`
/// then waited for the proxy to exit).
///
/// `assert_cmd`'s own timeout cannot catch that: it kills the command, but then waits for the end
/// of the captured streams, which the proxy still holds. So the command runs on a thread and
/// this test waits for its result with a deadline; on a miss it stops the proxy (which releases
/// the pipes and lets the thread end) and fails with the reason.
#[test]
fn proxy_start_with_captured_output_returns_while_the_proxy_keeps_running() {
    let (upstream, _seen) = fake_upstream();
    let port = free_port();
    let sb = Sandbox::new(&upstream, port);
    let _stop = StopProxy(&sb);
    let deadline = std::time::Duration::from_secs(60);

    let mut start = sb.tokred();
    start.args(["proxy", "start"]).timeout(deadline);
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(start.output());
    });
    let Ok(output) = receiver.recv_timeout(deadline) else {
        sb.run_detached(&["proxy", "stop"]);
        panic!(
            "`tokred proxy start` with captured output did not return within {deadline:?}: \
             the background proxy inherited the pipes of its caller"
        );
    };
    let output = output.unwrap();
    assert!(output.status.success(), "{output:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("The tokred proxy is running"), "{text}");

    sb.tokred()
        .args(["proxy", "status"])
        .timeout(deadline)
        .assert()
        .success()
        .stdout(predicate::str::contains("tokred proxy: running"));
    sb.tokred()
        .args(["proxy", "stop"])
        .timeout(deadline)
        .assert()
        .success()
        .stdout(predicate::str::contains("stopped"));
}

#[test]
fn the_proxy_prunes_records_older_than_retention_days_when_it_starts() {
    let (upstream, _) = fake_upstream();
    let port = free_port();
    let sb = Sandbox::new(&upstream, port);
    let path = sb.data().join("tokred.toml");
    let toml = std::fs::read_to_string(&path).unwrap();
    assert!(toml.contains("retention_days = 90"));
    std::fs::write(
        &path,
        toml.replace("retention_days = 90", "retention_days = 1"),
    )
    .unwrap();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let db = rusqlite::Connection::open(sb.data().join("tokred.db")).unwrap();
    for (id, ts) in [("old", 1_u64), ("new", now)] {
        db.execute(
            "INSERT INTO proxy_requests(id, ts, session_id, origin, model_requested, \
             model_effective, input, output, cache_write, cache_write_1h, cache_read, thinking, \
             bytes_in, bytes_out, upstream_ttfb_ms, total_ms, proxy_overhead_us, status, \
             tools_hash, system_hash) \
             VALUES (?1, ?2, 's', 'main', 'm', 'm', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 200, '', '')",
            rusqlite::params![id, i64::try_from(ts).unwrap()],
        )
        .unwrap();
    }

    sb.start_proxy();
    let mut ids = Vec::new();
    for _ in 0..50 {
        ids = db
            .prepare("SELECT id FROM proxy_requests ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        if ids == ["new"] {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(ids, ["new"], "the old record was pruned at start");
    sb.tokred().args(["proxy", "stop"]).assert().success();
}

/// `GET path` with an optional cookie, answered with `connection: close`.
fn http_get(port: u16, path: &str, cookie: Option<&str>) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let cookie = cookie.map_or_else(String::new, |c| format!("cookie: {c}\r\n"));
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{cookie}connection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The JSON body of a response.
fn json_body(response: &str) -> serde_json::Value {
    serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap()
}

/// What `tokred dashboard --no-open` prints: the address with the one-time token.
fn dashboard_url(sb: &Sandbox) -> String {
    let output = sb
        .tokred()
        .args(["dashboard", "--no-open"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("works once"), "{text}");
    text.lines()
        .find(|l| l.contains("/dashboard?token="))
        .unwrap()
        .trim()
        .to_owned()
}

/// Opens the dashboard the way the browser does: redeems the token and keeps the cookie.
fn sign_in(port: u16, url: &str) -> String {
    let path = url.split_once(&format!("127.0.0.1:{port}")).unwrap().1;
    let answer = http_get(port, path, None);
    assert!(answer.starts_with("HTTP/1.1 303"), "{answer}");
    let cookie = answer
        .lines()
        .find_map(|l| l.strip_prefix("set-cookie: "))
        .unwrap();
    cookie.split(';').next().unwrap().to_owned()
}

fn wait_for_exit(pid: u32) {
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
        assert!(gone, "the proxy process {pid} is still running");
    }
    #[cfg(not(unix))]
    let _ = pid;
}

fn read_pid(sb: &Sandbox) -> u32 {
    std::fs::read_to_string(sb.data().join("proxy/pid"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

const MAIN_BODY: &str = r#"{"model":"claude-sonnet-5-5","stream":true,"thinking":{"type":"adaptive"},"system":"Be brief.","tools":[{"name":"Read"}],"messages":[{"role":"user","content":"hello there"}]}"#;
const SESSION_HEADERS: &str = "x-api-key: sk-test-secret\r\nx-claude-code-session-id: dash-1\r\n";

/// The detail of the session `dash-1` once its turn has ended.
fn finished_detail(port: u16, cookie: &str) -> serde_json::Value {
    let mut detail = serde_json::Value::Null;
    for _ in 0..50 {
        let answer = http_get(port, "/dashboard/api/sessions/dash-1", Some(cookie));
        if answer.starts_with("HTTP/1.1 200") {
            detail = json_body(&answer);
            if detail["turns"][0]["in_flight"] == false {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    detail
}

#[test]
fn a_request_shows_up_in_the_dashboard_with_its_content_when_bodies_are_logged() {
    let (upstream, _) = fake_upstream();
    let port = free_port();
    let sb = Sandbox::new(&upstream, port);
    sb.set_proxy_key("log_bodies", "true");
    sb.start_proxy();

    // Nothing opens without signing in; the token in the address works once.
    assert!(http_get(port, "/dashboard/api/sessions", None).starts_with("HTTP/1.1 401"));
    let url = dashboard_url(&sb);
    let cookie = sign_in(port, &url);
    let path = url.split_once(&format!("127.0.0.1:{port}")).unwrap().1;
    assert!(
        http_get(port, path, None).starts_with("HTTP/1.1 401"),
        "single use"
    );

    let reply = post(port, SESSION_HEADERS, MAIN_BODY);
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");

    let detail = finished_detail(port, &cookie);
    let turn = &detail["turns"][0];
    assert_eq!(turn["in_flight"], false, "{detail}");
    assert_eq!(turn["origin"], "main");
    assert_eq!(turn["status"], 200);
    assert_eq!(turn["usage"]["output"], 25);
    assert_eq!(turn["stop_reason"], "end_turn");
    let sent = turn["sent"].as_array().unwrap();
    assert_eq!(sent[0]["role"], "system");
    assert_eq!(sent[0]["text"], "Be brief.");
    assert_eq!(sent[1]["text"], "hello there");
    assert!(turn["response"]["tool_uses"].is_array());
    assert_eq!(detail["session"]["id"], "dash-1");
    assert_eq!(detail["session"]["content_recorded"], true);

    let list = json_body(&http_get(port, "/dashboard/api/sessions", Some(&cookie)));
    assert_eq!(list["active"][0]["id"], "dash-1");

    let page = http_get(port, "/dashboard", Some(&cookie));
    assert!(
        page.starts_with("HTTP/1.1 200") && page.contains("/dashboard/app.js"),
        "{page}"
    );
    let report = http_get(port, "/metrics", Some(&cookie));
    assert!(
        report.starts_with("HTTP/1.1 200") && report.contains("<html"),
        "{report:.300}"
    );
    assert!(report.to_lowercase().contains("content-security-policy"));

    let pid = read_pid(&sb);
    sb.tokred().args(["proxy", "stop"]).assert().success();
    wait_for_exit(pid);

    // What was captured is in the database, and credentials are not.
    let db = rusqlite::Connection::open(sb.data().join("tokred.db")).unwrap();
    let (sent, response, tools): (Option<String>, Option<String>, String) = db
        .query_row(
            "SELECT sent_json, response_json, tools_json FROM captured_turns",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(sent.unwrap().contains("hello there"));
    assert!(response.is_some());
    assert_eq!(tools, "[]");
    let dump: String = db
        .query_row(
            "SELECT group_concat(coalesce(sent_json, '') || coalesce(response_json, ''), '') FROM captured_turns",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!dump.contains("sk-test-secret"));
}

#[test]
fn without_log_bodies_the_dashboard_shows_metadata_only() {
    let (upstream, _) = fake_upstream();
    let port = free_port();
    let sb = Sandbox::new(&upstream, port);
    sb.set_proxy_key("log_bodies", "false");
    sb.start_proxy();
    let cookie = sign_in(port, &dashboard_url(&sb));
    assert!(post(port, SESSION_HEADERS, MAIN_BODY).starts_with("HTTP/1.1 200"));

    let detail = finished_detail(port, &cookie);
    let turn = &detail["turns"][0];
    assert_eq!(turn["in_flight"], false, "{detail}");
    assert_eq!(turn["sent"], serde_json::Value::Null);
    assert_eq!(turn["response"], serde_json::Value::Null);
    assert_eq!(turn["usage"]["output"], 25, "metadata is still shown");
    assert_eq!(detail["session"]["content_recorded"], false);

    let pid = read_pid(&sb);
    sb.tokred().args(["proxy", "stop"]).assert().success();
    wait_for_exit(pid);
    let db = rusqlite::Connection::open(sb.data().join("tokred.db")).unwrap();
    let content: i64 = db
        .query_row(
            "SELECT count(*) FROM captured_turns WHERE sent_json IS NOT NULL OR response_json IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(content, 0, "no content was stored");
    let rows: i64 = db
        .query_row("SELECT count(*) FROM captured_turns", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1, "the metadata row is there");
}

#[test]
fn the_dashboard_needs_a_running_proxy() {
    let (upstream, _) = fake_upstream();
    let sb = Sandbox::new(&upstream, free_port());
    sb.tokred()
        .args(["dashboard", "--no-open"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not running"));
}

#[test]
fn an_open_event_stream_does_not_keep_the_proxy_from_stopping() {
    let (upstream, _) = fake_upstream();
    let port = free_port();
    let sb = Sandbox::new(&upstream, port);
    sb.start_proxy();
    let cookie = sign_in(port, &dashboard_url(&sb));

    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    let request = format!(
        "GET /dashboard/events HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\ncookie: {cookie}\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut seen = String::new();
    let mut buffer = [0_u8; 1024];
    while !seen.contains("retry:") {
        let read = stream.read(&mut buffer).unwrap();
        assert!(read > 0, "the stream ended early: {seen}");
        seen.push_str(&String::from_utf8_lossy(&buffer[..read]));
    }
    assert!(seen.starts_with("HTTP/1.1 200"), "{seen}");
    assert!(seen.to_lowercase().contains("text/event-stream"));

    // A request while the stream is open produces events.
    assert!(post(port, SESSION_HEADERS, MAIN_BODY).starts_with("HTTP/1.1 200"));
    while !seen.contains("event: turn_end") {
        let read = stream.read(&mut buffer).unwrap();
        assert!(read > 0, "the stream ended early: {seen}");
        seen.push_str(&String::from_utf8_lossy(&buffer[..read]));
    }
    assert!(seen.contains("event: turn_start"));

    let pid = read_pid(&sb);
    sb.tokred().args(["proxy", "stop"]).assert().success();
    // The server ends the stream: the read reaches the end of the connection.
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => panic!("the stream was not closed by the proxy: {error}"),
        }
    }
    wait_for_exit(pid);
}
