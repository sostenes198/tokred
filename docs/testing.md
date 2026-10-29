**English** | [Português](testing.pt-BR.md)

# Testing tokred locally

Two ways to test: [run the automated tests](#1-automated-tests), and [try the tool by hand](#2-try-it-by-hand) without touching your real Claude Code setup. How the tests are organised and what they must follow is in [development](development.md); how to ship a release is in [release](release.md).

## 1. Automated tests

Requirements: Rust stable (`rust-toolchain.toml` installs `rustfmt` and `clippy`), plus `cargo install cargo-deny cargo-llvm-cov` for the full gate. No network and no real Claude Code are needed: every test uses temporary folders and fake servers.

```sh
cargo test --workspace --no-fail-fast        # everything
cargo test -p tokred-core                    # one crate
cargo test -p tokred --test proxy_e2e        # one test file (the real binary, proxy end to end)
cargo test -p tokred a_stopped_proxy         # tests whose name contains this text
cargo test -p tokred -- --nocapture          # show what tests print
```

| Where | What it proves |
|---|---|
| `#[cfg(test)]` modules | Pure logic: classification, pricing, routing decisions, compression, config |
| `crates/<crate>/tests/` | Real SQLite files, real temp folders, the proxy against a fake upstream (forwarding, streaming, gzip) |
| `crates/tokred-cli/tests/e2e.rs` | The real `tokred` binary: `configure` (port and content flags), hooks, `doctor`, `unconfigure`, `uninstall` in a sandbox |
| `crates/tokred-proxy/tests/network.rs` | The corporate-network path: a fake HTTP proxy answering `CONNECT` in front of a fake HTTPS upstream signed by a throwaway authority (`rcgen`), `NO_PROXY`, refused tunnels, bad values; `network_logs.rs` checks that credentials never reach the logs |
| `crates/tokred-cli/tests/proxy_e2e.rs` | The real binary running the proxy: forwarding, recording, `Host` check, `tokred claude`, advice commands, the process exiting after `proxy stop`, `proxy start` with captured output (on Windows a detached proxy used to keep the caller waiting; the first CI run on Windows is the check of the `Start-Process` launch) |
| `tests/installer/install_test.sh`, `install_test.ps1` | The install and uninstall scripts against a fake release (see below) |

### The full gate (what CI runs)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --no-fail-fast
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
cargo +1.89 check --workspace --all-targets --locked
```

### Coverage

The target is 100% of lines.

```sh
cargo llvm-cov --workspace --no-fail-fast --summary-only
cargo llvm-cov --workspace --no-fail-fast --lcov --output-path lcov.info   # `DA:<line>,0` = never ran
```

### Flaky tests and hangs

- Run a suspicious suite a few times: `for i in 1 2 3; do cargo test -p tokred --no-fail-fast; done`.
- macOS has no `timeout`; bound a run with `perl -e 'alarm 280; exec @ARGV' cargo test -p tokred`.
- After an interrupted run, look for a proxy left behind: `pgrep -fl "proxy start --foreground"` and stop it with `pkill -f "proxy start --foreground"`.
- Snapshots (`insta`): `INSTA_UPDATE=always cargo test`, then review `git diff -- '*.snap'`.

CI runs the suite on Linux, macOS and Windows; a test that passes only on your OS is not done.

### The install scripts

`tests/installer/` tests `install/install.sh` and `install/uninstall.sh` (CI: ubuntu and macOS) and the PowerShell pair (CI: Windows). They build a **fake release**: the just-built binary packed the way `dist` packs it (`tokred-<target>/tokred` inside `tokred-<target>.tar.gz`, or `.zip` with `tokred.exe`) plus its `.sha256`, served through `TOKRED_RELEASE_BASE_URL=file://...`, with a throwaway `HOME`, `TOKRED_HOME`, data folder and Claude folder. No network, nothing real is touched.

```sh
cargo build -p tokred && sh tests/installer/install_test.sh
shellcheck install/*.sh tests/installer/*.sh        # what the CI `shellcheck` job runs
```

The shell test covers: install, the `PATH`/`claude` block written once and byte-identical on a second run, a new shell finding `tokred --version`, the wrapper end to end in `sh`, `bash`, `dash` and `zsh` when present (a fake `claude` that records its arguments and a fake `tokred` that records that `initialize` ran first; arguments with spaces, quotes, an empty string, `*` and `$HOME` arrive intact; `claude` still runs without `tokred`; `command claude` bypasses it), an existing `alias claude` in zsh, `--no-alias`, `--no-modify-path`, `TOKRED_NO_ALIAS=1`, fish and zsh files, uninstall (everything gone, other lines kept, `--purge`), a wrong checksum, a missing release, a bad port and unknown flags. The wrapper test uses a fake `tokred`; `tokred initialize` itself is tested in its own crate.

`install_test.ps1` is the same story for Windows; it edits the runner's user `PATH` and PowerShell profile and restores them, so outside CI it needs `-AllowRealProfile`. To try the scripts by hand without a release, see [release](release.md).

## 2. Try it by hand

Use a sandbox so nothing is written to your real `~/.claude` or data folder. Run this in every new terminal you use for testing:

```sh
cargo build
export PATH="$PWD/target/debug:$PATH"
export TOKRED_DATA_DIR=$(mktemp -d) CLAUDE_CONFIG_DIR=$(mktemp -d) TOKRED_OFFLINE=1
mkdir "$CLAUDE_CONFIG_DIR/projects"
tokred configure --yes && tokred doctor
```

`doctor` should show the hooks and agents installed in the sandbox. When you are done: `tokred uninstall`, then close the terminal (the exports die with it).

### A hook

```sh
echo '{"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"Cargo.lock"}}' \
  | tokred hook guard-read; echo "exit=$?"       # an allowed call prints nothing and exits 0
tokred doctor --bench                            # hook latency; TOKRED_BENCH_RUNS=3 for a quick run
```

### The proxy, without an API key

Point the proxy at a fake upstream that returns a message with token counts.

```sh
# Terminal A: the fake API on port 9000
python3 -I - <<'PY'
import itertools, json, http.server
ids = itertools.count(1)
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length", 0)))
        body = json.dumps({"id": f"msg_{next(ids)}", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
                           "content": [{"type": "text", "text": "hi"}], "stop_reason": "end_turn",
                           "usage": {"input_tokens": 120, "output_tokens": 15}}).encode()
        self.send_response(200); self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
http.server.HTTPServer(("127.0.0.1", 9000), H).serve_forever()
PY
```

```sh
# Terminal B (with the sandbox exports above)
sed -i '' 's#^upstream = .*#upstream = "http://127.0.0.1:9000"#' "$TOKRED_DATA_DIR/tokred.toml"   # Linux: sed -i without ''
tokred proxy start && tokred proxy status

# a small request (no tools, one message: tokred sees it as "auxiliary"), then a "subagent" one (it carries an agent id)
curl -s http://127.0.0.1:1029/v1/messages -H 'x-api-key: test' -H 'x-claude-code-session-id: s1' \
  -H 'content-type: application/json' -d '{"model":"claude-sonnet-4-5","max_tokens":16,"messages":[{"role":"user","content":"hi"}]}'
curl -s http://127.0.0.1:1029/v1/messages -H 'x-api-key: test' -H 'x-claude-code-session-id: s1' \
  -H 'x-claude-code-agent-id: a1' -H 'content-type: application/json' \
  -d '{"model":"claude-sonnet-4-5","max_tokens":16,"tools":[{"name":"Grep","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":"find where main is defined"}]}'

tokred routing status        # both requests were classified; mode is observe, so the models were kept
tokred proxy stop
```

What to check:

- the replies you get are exactly what the fake returned (the proxy changes nothing in `observe`);
- `sqlite3 "$TOKRED_DATA_DIR/tokred.db" "select origin, model_requested, model_effective, input, output from proxy_requests"` shows two rows (`auxiliary` and `subagent`) with 120 input and 15 output tokens;
- `tokred stats` shows a **Proxy** section counting the two requests (`2 messages measured by the proxy`), and `tokred stats --by origin` lists `auxiliary` and `subagent`;
- a request with `Host: evil.example` is refused: `curl -s -o /dev/null -w '%{http_code}\n' -H 'Host: evil.example' http://127.0.0.1:1029/v1/messages -d '{}'` prints `421`;
- stopping really ends the process: `pgrep -fl "proxy start --foreground"` prints nothing.

### The proxy with the real Claude Code

You need your own login (an API key, or `claude` logged into a subscription). Using a throwaway `CLAUDE_CONFIG_DIR` means logging in again inside it; that is intended.

```sh
tokred claude                # same as `claude`, through the proxy, for this run only
# work for a minute, with at least one subagent (e.g. ask it to "explore the repo")
tokred routing status        # decisions for the subagent requests
tokred suggest               # advice, once there were several simple prompts
tokred proxy status
```

To compare with what Claude Code itself recorded: the `input`, `output` and cache figures of a request in `proxy_requests` must match the `usage` of the same message in `$CLAUDE_CONFIG_DIR/projects/**/*.jsonl`.

### Routing and compression switches

```sh
tokred routing on            # [routing] mode = "enforce"; restart the proxy to apply
tokred routing off
tokred compression on        # registers the tokred_retrieve MCP server in ~/.claude.json (backup .tokred.bak)
tokred compression status
tokred compression off
```

`compression on` is the only command that edits Claude Code's user config. When the sandbox folder has no `.claude.json`, tokred falls back to `~/.claude.json`, so point `HOME` to a temporary folder too (`export HOME=$(mktemp -d)`) before trying it.

### Status line

```sh
echo '{"session_id":"s1"}' | tokred statusline     # the pending suggestion, or the session figures
```

### Clean up

```sh
tokred proxy stop; tokred uninstall
# and stop the fake API with Ctrl-C in terminal A
```
