**English** | [Português](architecture.pt-BR.md)

# Architecture

How tokred is built, for people who change it: the crates, the rules they follow, how a hook and a proxy request flow, and what is stored locally. For building and testing see [development](development.md); for why some choices were made see [decisions](decisions.md).

The design is a clean architecture for Rust: pure business rules at the centre, I/O at the edges, dependencies pointing inwards, and a single composition point (the binary).

```text
tokred (crates/tokred-cli) ──► every other crate
                                      │
  use-case crates ─┐                  ▼
  adapter crates  ─┴────────────► tokred-core   (domain types, config model, ports; no I/O)
```

Use-case and adapter crates depend on `tokred-core` and never on each other (only their tests may use `tokred-store`). Only the binary knows all of them.

## Crates

| Crate | Role |
|---|---|
| `tokred-core` | Domain: types, errors, the `tokred.toml` model, path rules, the hook event model, the tiered-cache policy and the ports (traits such as `Clock`, `FileSystem` and the repositories). **No I/O** |
| `tokred-install` | `configure`, `unconfigure`, `doctor`: pure settings merge, agent templates (`templates/`, embedded in the binary) and install rules; its `fs` module is the `std::fs` adapter |
| `tokred-hooks` | The seven hook handlers, the time guard and the benchmark payloads. Fail-open |
| `tokred-metrics` | Pure: transcript ingestion, the merge of transcript and proxy usage, cost, savings, projection, latency and the routing and guard summaries |
| `tokred-pricing` | Price list parsing, the embedded fallback snapshot, sync use cases and the LiteLLM adapter |
| `tokred-update` | Install-method detection, version check, self-update with checksum verification, GitHub adapter |
| `tokred-audit` | Fixed-context audit: `CLAUDE.md` with imports, rules, skills, agents, MCP usage |
| `tokred-report` | The offline HTML report: model, SVG charts, template, atomic write |
| `tokred-store` | Adapter: SQLite (`rusqlite`), versioned migrations, repository implementations |
| `tokred-proxy` | Adapter: the local proxy (`hyper`, `tokio`, `rustls`). Listens on loopback, forwards with streaming, measures usage and latency, runs the hook pipeline defined in `tokred-core`, and serves the [dashboard](dashboard.md). The only crate that depends on `hyper` and `tokio`. It reads history through the `DashboardSource` port, which the binary implements over `tokred-store` and `tokred-report` |
| `tokred-routing` | Pure: classifies subagent and auxiliary requests and picks a cheaper model when it is safe. The main loop's model is never changed |
| `tokred-suggest` | Pure: summaries of the main loop's prompts and suggestions of a cheaper model. It only advises |
| `tokred-compress` | Pure: reversible compression of large tool results, the response-cache measurement and the `tokred_retrieve` MCP server logic |
| `tokred` (`crates/tokred-cli`) | The binary. Parses arguments, builds the real adapters, calls the use cases, formats output and decides the exit code. The only crate that uses `anyhow` and `println!`. The package is named `tokred`; its folder keeps the `tokred-cli` name |

`tokred-routing`, `tokred-suggest` and `tokred-compress` plug into the proxy through the `ProxyHook` pipeline defined in `tokred-core`; `tokred-proxy` runs them over HTTP and the binary wires everything and runs it with `tokred proxy start --foreground`. See [proxy](proxy.md).

## Rules

- Core logic is **pure functions** over data (`merge_settings`, `remove_tokred`, `plan_agent`, `merge_config`, `merge_usage`...). I/O enters through traits passed as parameters, and the one place that builds the real implementations is the binary.
- Where a crate does I/O itself (`tokred-install::fs`, the HTTP clients of `tokred-pricing` and `tokred-update`), it is in a module of its own behind a trait, so the logic around it is tested with fakes.
- `Clock` exists so TTLs, the daily version check and per-day metrics are testable without waiting.
- Inside the binary, terminal I/O goes through a small `Io` seam (interactive flag, input, output) and command output is rendered by pure functions, so prompts and reports are unit-tested without a terminal.
- Errors are `thiserror` enums per crate; `anyhow` only in the binary.
- Lints: `unsafe_code = forbid`; clippy `pedantic` as a warning; `unwrap`, `expect`, `panic`, `todo` and `dbg!` denied outside tests (`Cargo.toml`, `[workspace.lints]`).

## How a hook runs

1. Claude Code runs `"<path>/tokred" hook <event>` and writes JSON to stdin.
2. `main` handles `hook` before argument parsing, so even an unknown event exits 0. A hook never blocks Claude Code: exit code 2 would mean "block".
3. The handler (`tokred-hooks`) runs inside `catch_unwind`, behind a watchdog (`TOKRED_HOOK_GUARD_MS`, default 2 s). On error, panic or timeout it logs to `<data>/logs/` and prints nothing.
4. A decision, if any, is printed as JSON on stdout (`hookSpecificOutput`, `permissionDecision`, `additionalContext`, `systemMessage`) and the process exits 0.
5. The database row of the run (`hook_events`) is written last. A hook never creates the data folder or the database: a purged installation stays purged.

## How a proxy request flows

1. The proxy accepts the connection on loopback and checks the `Host` header (`421` otherwise).
2. `/__tokred/*` are admin endpoints: `health` is open, `status`, `shutdown` and `dashboard-token` need the secret.
3. `/dashboard` and `/metrics` are answered locally, behind the browser sign-in. Everything else is forwarded.
4. For `POST /v1/messages` with a JSON body, the hook pipeline runs: response-cache measurement, routing, compression, then prompt summaries and suggestions. A hook that errors or panics discards all its changes and the original bytes are forwarded.
5. The request goes upstream over `hyper` and `rustls` (through the HTTP proxy tunnel and extra certificate authorities when configured), and the response streams back. A tap reads `usage`, the stop reason and the tool calls as the bytes pass, without buffering.
6. If the API rejects a rewritten request (400, 404 or 422), the original is sent instead.
7. When the response ends, one record (the request row, routing decision, summaries) is handed to a writer thread that writes it to the store, so database work never runs on the request path. When the writer's queue is full the record is dropped and logged, never delaying the request. The live dashboard state and its event bus are updated from the same data.

## The local store

tokred keeps its state in one SQLite file, `<data>/tokred.db`, opened with `journal_mode=WAL` and `busy_timeout=2000`.

The schema is versioned with `PRAGMA user_version`. Migrations are SQL strings in `crates/tokred-store/src/migrations.rs`; each runs in its own transaction, and a database created by a newer tokred is refused rather than downgraded. To change the schema, append a migration; never edit one that has been released.

| Table | Holds |
|---|---|
| `sessions` | Session id, working directory, first and last seen, last reset (`clear` or `compact`) |
| `classifications` | Actionable prompt class per session (never `other`) |
| `file_reads` | `(scope, path, hash, offset, limit)` reads and estimated bytes; the scope is the session id, plus `/<agent_id>` inside a subagent |
| `hook_events` | Every hook run: decision, reason, bytes saved, duration in microseconds, outcome (`ok`, `error`, `panic`, `timeout_guard`) |
| `cache_entries` | The tiered cache (below) |
| `installed_files` | Files tokred installed, with blake3 hash and version |
| `pricing` | USD per million tokens, per model |
| `usage`, `ingest_state` | Per-message token usage from transcripts, and the incremental ingestion offsets |
| `meta` | Small key/value state (version check, price sync, benchmark, `tokred_version` marker, compression breaker) |
| `proxy_requests` | One row per request the proxy saw: models, tokens, latency, stop reason, tools, cache lifetime, routing mode, cache-miss cause |
| `routing_decisions`, `prompt_summaries`, `suggestions`, `shadow_savings` | Routing decisions, summaries of the main loop's prompts, suggestions and measured savings |
| `stored_results`, `compressed_blocks` | Originals of compressed tool results and the replacements sent, so a replacement is identical every turn |
| `captured_turns` | What the dashboard shows of a turn; the content columns stay empty unless `[proxy] log_bodies` is on |

### Tiered cache (`cache_entries`)

| Tier | Enters when | Expires |
|---|---|---|
| L1 | First write | `l1_ttl` (default 4 h) after creation |
| L2 | `hits >= l2_hits` (default 3) | `l2_ttl` (default 24 h), renewed on every hit |
| L3 | `hits >= l3_hits` (default 10) | Never (`expires_at = NULL`) |

- Keys include the hash of the source content, so a changed source invalidates its entries: `<kind>:<hash>:<subject>`. For file counts the content is the repository root's modification time and the threshold, the cheapest signal that the repository changed.
- Promotion happens on read (`cache_get`): it counts a hit, re-evaluates the tier and renews L2 expiry. `cache_peek` reads without counting. An expired entry is a miss.
- An unreadable `l1_ttl` or `l2_ttl` (anything but a positive number plus `s`, `m`, `h` or `d`) falls back to its default instead of breaking a hook.
- The policy is pure functions in `tokred-core` (`next_tier`, `expires_at`, `is_expired`, `record_hit`); the store only loads and saves rows.
- `tokred cache stats` shows entries, expired entries, hits and bytes per tier; `tokred cache prune` removes expired ones; nothing else prunes them, and a hook never does.
- It holds the per-root file counts of `guard-search` (kind `repo_files`), the class of each prompt seen by `route` (kind `classification`, keyed by the blake3 hash of the prompt and a rules version) and the "already warned" markers of `session-check` (kind `session_warn`).

## Data folder

| Platform | Default location |
|---|---|
| Linux | `~/.local/share/tokred` |
| macOS | `~/Library/Application Support/tokred` |
| Windows | `%APPDATA%\tokred` |

`TOKRED_DATA_DIR` overrides it. Inside:

| Path | What |
|---|---|
| `tokred.db` | The SQLite database |
| `tokred.toml` | Configuration ([configuration](configuration.md)) |
| `logs/` | tokred's own logs, rotated daily, 7 days kept |
| `reports/` | HTML reports written by `tokred metrics` |
| `proxy/secret` | The proxy's admin secret (owner-only permissions on Unix) |
| `proxy/pid` | The pid and port of the background proxy |
| `proxy/base_urls` | The proxy URLs tokred has written to Claude Code's settings, one per line, so a changed port is followed |

The binary and the `claude` wrapper live outside the data folder, under `~/.tokred/bin` (Windows `%LOCALAPPDATA%\tokred\bin`); see [installation](installation.md).
