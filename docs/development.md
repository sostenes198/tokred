**English** | [Português](development.pt-BR.md)

# Development

How to build, run, test and try tokred from the source tree. How the code is organised is in [architecture](architecture.md); how to ship a release is in [release](release.md); the contribution workflow is in [CONTRIBUTING](../CONTRIBUTING.md).

## Requirements

- Rust 1.89 or newer (the workspace MSRV, checked in CI). `rust-toolchain.toml` selects stable with `rustfmt` and `clippy`.
- For the full local gate: `cargo install cargo-deny cargo-llvm-cov`.
- For the shell tests: `shellcheck`. The release smoke test also needs Python 3.7+ and `curl`. The documentation check needs Python 3.

## Build and run

```sh
cargo build
```

```sh
cargo run -q -- doctor
```

`cargo run -q -- <command>` runs any command. A binary built from source works from wherever it is, but `tokred update` tells you to reinstall with the script: only a binary in the install folder updates itself. To run the background proxy, use the built binary and not `cargo run`, so the proxy outlives the build.

## The quality gate

CI runs these on every pull request; run them before you push.

```sh
cargo fmt --all --check
```

```sh
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

```sh
cargo test --workspace --all-features --no-fail-fast
```

```sh
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

```sh
cargo deny check
```

```sh
cargo +1.89 check --workspace --all-targets --locked
```

```sh
python3 scripts/check-docs.py
```

`cargo +1.89 check` is the MSRV check: use the version written in `[workspace.package] rust-version` of `Cargo.toml`. `check-docs.py` checks the documentation: each document has its `.pt-BR.md` twin with the same headings and code blocks, links resolve, and a few house rules hold (see its header).

### Coverage

Cover the lines you add. CI measures coverage and uploads `lcov.info`; it does not fail on a drop, so check it yourself.

```sh
cargo llvm-cov --workspace --all-features --summary-only
```

To list the lines that never run:

```sh
cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info
```

In `lcov.info`, `DA:<line>,0` is a line that never ran. `cargo llvm-cov report --text` prints annotated source, where `^0` marks a region that never ran. The per-line views are the reference: LLVM's region and function summary reads a little lower because every `?` has an error branch, and forcing each I/O failure would add noise, not safety.

## Tests

Run everything, one crate, one file or one test:

```sh
cargo test --workspace --no-fail-fast
```

```sh
cargo test -p tokred-core
```

```sh
cargo test -p tokred --test proxy_e2e
```

```sh
cargo test -p tokred a_stopped_proxy
```

| Layer | Where | What it proves |
|---|---|---|
| Unit | `#[cfg(test)]` modules | Pure logic: classification, pricing, routing, compression, config, merge functions |
| Integration | `crates/<crate>/tests/` | Real SQLite files and temp folders; `tokred-proxy` against a fake upstream (forwarding, streaming, gzip, dashboard); the corporate-network path (`network.rs`: a fake HTTP proxy answering `CONNECT` in front of a fake HTTPS upstream signed by a throwaway authority, `NO_PROXY`, refused tunnels; `network_logs.rs`: credentials never reach the logs) |
| Property | `proptest`, for example `crates/tokred-install/tests/merge_props.rs` | Configuring twice equals once; configure then unconfigure restores the original; foreign hooks survive; merging usage again changes nothing |
| End to end | `crates/tokred-cli/tests/` | The real binary through `assert_cmd`: `e2e.rs` (configure, hooks, doctor, unconfigure, uninstall in a sandbox), `proxy_e2e.rs` (the proxy process: forwarding, recording, `Host` check, `tokred claude`, stopping, `proxy start` with captured output, retention, the live dashboard), `initialize_e2e.rs`, `dashboards.rs` (the terminal dashboards, the HTML report, the cache, `doctor --bench`), `commands.rs` (the commands that use the network, against a local server) |
| Snapshot | `insta`, next to the tests | Merged `settings.json`, `doctor --json`, the terminal sections, the JSON shape of the audit |
| Installer | `tests/installer/install_test.sh`, `install_test.ps1` | The install and uninstall scripts against a fake release served from disk, in a throwaway home |
| Release script | `tests/release_scripts/test.sh` | `scripts/release.sh` in a throwaway repository with a fake `gh`. Not run by CI: run it when you change the script |
| Release smoke | `tests/release/smoke.sh`, `smoke.ps1` | The whole product installed from a **published** release; see [release](release.md#release-smoke-test) |

### Rules

- No test touches the real `~/.claude`, the real data folder or the network. End-to-end tests set `HOME`, `CLAUDE_CONFIG_DIR` and `TOKRED_DATA_DIR` to temporary folders (one with a space in its name), and the proxy is tested against local fake servers.
- Every bug fix comes with a test that reproduces the bug.
- Files under `tests/` start with `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`, because clippy's `allow-*-in-tests` options cover only `#[cfg(test)]` code.
- Fixtures live in `tests/fixtures/` (`settings.json` variants, including one with another hook tool's hooks, and hook payloads in `tests/fixtures/hooks/`). Add one whenever you find a real-world shape the merge must handle.

### Snapshots

```sh
INSTA_UPDATE=always cargo test
```

Review what changed before committing: `git diff -- '*.snap'`.

### The install scripts

The installer tests build a **fake release**: the just-built binary packed the way `dist` packs it (`tokred-<target>/tokred` inside `tokred-<target>.tar.gz`, or a `.zip` with `tokred.exe`) plus its `.sha256`, served through `TOKRED_RELEASE_BASE_URL=file://...`, with a throwaway `HOME`, `TOKRED_HOME`, data folder and Claude folder.

```sh
cargo build -p tokred
```

```sh
sh tests/installer/install_test.sh
```

```sh
shellcheck install/*.sh tests/installer/*.sh tests/release/*.sh
```

`install_test.ps1` does the same on Windows. It edits the runner's user `PATH` and PowerShell profile and restores them, so outside CI it needs `-AllowRealProfile`. The scripts are kept thin on purpose: every prompt lives in Rust (`tokred proxy enable`, `tokred uninstall`), where it is unit-tested through the `Io` seam.

### Flaky tests and hangs

- Run a suspicious suite a few times: `for i in 1 2 3; do cargo test -p tokred --no-fail-fast; done`.
- macOS has no `timeout`; bound a run with `perl -e 'alarm 280; exec @ARGV' cargo test -p tokred`.
- After an interrupted run, look for a proxy left behind with `pgrep -fl "proxy start --foreground"`. If you also run tokred for real, that proxy shows up in the list too: stop only the others, by their pid with `kill <pid>`.

## Try it by hand

Use a sandbox so nothing is written to your real `~/.claude` or data folder. Run these in every terminal you use for testing; the exports die with the terminal.

```sh
cargo build
```

```sh
unset ANTHROPIC_BASE_URL
```

```sh
export PATH="$PWD/target/debug:$PATH" TOKRED_DATA_DIR=$(mktemp -d) CLAUDE_CONFIG_DIR=$(mktemp -d) TOKRED_OFFLINE=1
```

```sh
mkdir "$CLAUDE_CONFIG_DIR/projects"
```

```sh
tokred configure --yes && tokred doctor
```

`doctor` shows the hooks and agents installed in the sandbox. The `unset` keeps a gateway from your own shell out of the sandbox. `TOKRED_OFFLINE=1` turns off the version check and the price sync. When you are done, stop the proxy if you started one (`tokred proxy stop`) and close the terminal. Do not run `tokred uninstall` here: it also removes the service, the `PATH` entry and the `claude` wrapper of your real home.

### A hook

A hook reads JSON on stdin, prints its decision (if any) as JSON and exits 0 whatever happens. It never creates the data folder or the database. The `guard-read` hook denies lockfiles, so this prints a `permissionDecision` of `deny` and `exit=0`:

```sh
echo '{"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"Cargo.lock"}}' | tokred hook guard-read; echo "exit=$?"
```

An allowed call prints nothing:

```sh
echo '{"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"README.md"}}' | tokred hook guard-read; echo "exit=$?"
```

`tokred doctor --bench` measures hook latency; `TOKRED_BENCH_RUNS=3` makes it quick. Before you implement a hook, read the [official hooks documentation](https://code.claude.com/docs/en/hooks) for the exact input and output format, and record any divergence in [decisions](decisions.md).

### The proxy, without an API key

Point the proxy at a fake upstream that returns a message with token counts. In terminal A, start the fake API on port 9000:

```sh
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

In terminal B (with the sandbox exports), point the proxy at it and start it on a port of your own, so a tokred you use for real on its default port is not involved:

```sh
perl -pi -e 's#^upstream = .*#upstream = "http://127.0.0.1:9000"#' "$TOKRED_DATA_DIR/tokred.toml"
```

```sh
tokred proxy enable --port 1050 --yes && tokred proxy status
```

Send a small request (no tools, one message: tokred sees it as `auxiliary`), then one that carries an agent id (a `subagent`):

```sh
curl -s http://127.0.0.1:1050/v1/messages -H 'x-api-key: test' -H 'x-claude-code-session-id: s1' -H 'content-type: application/json' -d '{"model":"claude-sonnet-4-5","max_tokens":16,"messages":[{"role":"user","content":"hi"}]}'
```

```sh
curl -s http://127.0.0.1:1050/v1/messages -H 'x-api-key: test' -H 'x-claude-code-session-id: s1' -H 'x-claude-code-agent-id: a1' -H 'content-type: application/json' -d '{"model":"claude-sonnet-4-5","max_tokens":16,"tools":[{"name":"Grep","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":"find where main is defined"}]}'
```

Check:

- the replies are exactly what the fake returned (in `observe` the proxy changes nothing);
- `tokred routing status` lists two decisions, both `kept` (nothing is changed in `observe`);
- `sqlite3 "$TOKRED_DATA_DIR/tokred.db" "select origin, model_requested, model_effective, input, output from proxy_requests"` shows two rows (`auxiliary` and `subagent`) with 120 input and 15 output tokens;
- `tokred stats` has a **Proxy** section counting two requests, and `tokred stats --by origin` lists both origins;
- `tokred dashboard --no-open` prints a one-time address for the [dashboard](dashboard.md); open it in a browser to see the session;
- a wrong `Host` is refused: `curl -s -o /dev/null -w '%{http_code}\n' -H 'Host: evil.example' http://127.0.0.1:1050/v1/messages -d '{}'` prints `421`.

Then stop everything: `tokred proxy stop` must say the proxy stopped, `tokred proxy status` must say it is not running, and stop the fake API with Ctrl-C in terminal A.

### The proxy with the real Claude Code

You need your own login (an API key, or `claude` logged into a subscription). A throwaway `CLAUDE_CONFIG_DIR` means logging in again inside it; that is intended.

```sh
tokred proxy enable --port 1050 --yes
```

```sh
tokred claude
```

Work for a minute with at least one subagent (for example ask it to "explore the repo"), then look at `tokred routing status`, `tokred suggest` and `tokred proxy status`. The `input`, `output` and cache figures of a request in `proxy_requests` must match the `usage` of the same message in `$CLAUDE_CONFIG_DIR/projects/**/*.jsonl`.

`tokred compression on` registers the `tokred_retrieve` MCP server in Claude Code's user config (`~/.claude.json`, with a backup next to it). Only `tokred compression on`, `tokred compression off` and `tokred uninstall` edit that file; when the sandbox folder has no `.claude.json` tokred falls back to the one in your home, so also `export HOME=$(mktemp -d)` before trying them.

## Continuous integration

| Workflow and job | What it runs |
|---|---|
| CI `fmt`, `clippy`, `docs`, `deny`, `msrv`, `documentation` | The checks of the quality gate, on Linux (`docs` is `cargo doc`, `documentation` is `scripts/check-docs.py`) |
| CI `test` | `cargo test` on Linux, macOS and Windows |
| CI `coverage` | `cargo llvm-cov`, uploading `lcov.info` |
| CI `shellcheck` | `install/*.sh`, `tests/installer/*.sh`, `tests/release/*.sh` |
| CI `installer`, `installer-windows` | The installer tests on Ubuntu, macOS and Windows |
| `Release` | Builds the archives on a tag; see [release](release.md) |
| `Release smoke` | Installs the published release on real systems; see [release](release.md) |
| `Cut a release` | Runs `scripts/release.sh` by hand from the Actions tab; see [release](release.md) |

A test that passes only on your operating system is not done.

## Conventions that tripped people

- **`cfg(unix)`-only helpers.** A helper used only by a `#[cfg(unix)]` test needs `#[cfg(unix)]` too, or it is dead code and `-D warnings` fails the Windows build.
- **Windows detached processes.** A child inherits the parent's inheritable handles, so a captured `tokred proxy start` would wait for the proxy. The proxy is started through `Start-Process`, which does not inherit them (see [decisions](decisions.md)). Tests that run `proxy start` with captured output enforce a deadline on a thread rather than trusting the command's own timeout.
- **macOS has no `timeout`:** use the `perl` form above.
- **Line endings.** `.gitattributes` keeps text files LF and PowerShell scripts CRLF on every checkout.
- **Price snapshot.** `scripts/update-price-snapshot.sh` regenerates the embedded price list.
