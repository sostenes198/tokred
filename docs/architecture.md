**English** | [Português](architecture.pt-BR.md)

# Architecture

Idiomatic clean architecture for Rust: **pure business rules at the centre, I/O at the edges**, dependencies always pointing inwards, and a single composition point (the binary).

```
tokred (cli) ──► use cases (install, hooks, metrics, pricing, audit, update, report) ──► tokred-core
     │                                                                                       ▲
     └──► adapters (tokred-store, fs/http modules) ── implement the traits (ports) of ───────┘
```

## Crates

| Crate | Role |
|---|---|
| `tokred-core` | Domain: types, errors, config model, path rules, hook event model, and the ports (`Clock`, `FileSystem`, `ManifestRepo`; more arrive with their phases). **No I/O.** |
| `tokred-install` | Use cases `configure`, `unconfigure`, `doctor`: pure settings merge, agent templates and install rules. Contains the `std::fs` adapter (`fs` module) |
| `tokred-hooks` | The six hook handlers, the time guard and the benchmark payloads (fail-open) |
| `tokred-metrics` | Transcript ingestion, cost, savings, projection, latency and routing/guard summaries (pure) |
| `tokred-pricing` | Price list parsing, embedded fallback snapshot, sync use cases and the LiteLLM adapter |
| `tokred-update` | Install-method detection, version check, self-update with checksum verification, GitHub adapter |
| `tokred-audit` | Fixed-context audit: `CLAUDE.md` with imports, rules, skills, agents, MCP usage |
| `tokred-report` | The offline HTML report: model, SVG charts, template, atomic write |
| `tokred-store` | Adapter: SQLite (`rusqlite`), versioned migrations, repository implementations |
| `tokred-proxy` | Adapter: the optional local proxy (`hyper`, `tokio`, `rustls`): listens on loopback, forwards with streaming, measures usage and latency, and runs the proxy hook pipeline defined in `tokred-core`. The **only** crate that depends on `hyper` and `tokio`. It also serves the live [dashboard](dashboard.md): an in-memory view of the sessions, an event bus (server-sent events) and the sign-in, reading history through the `DashboardSource` port of `tokred-core`, which the binary implements over `tokred-store` and `tokred-report` |
| `tokred-routing` | Model routing policy: classifies subagent and auxiliary requests and picks a cheaper model when it is safe. Pure; the main loop's model is never changed |
| `tokred-suggest` | Summaries of the main loop's prompts and suggestions of a cheaper model. Pure; it only advises |
| `tokred-compress` | Reversible compression of large tool results and the `tokred_retrieve` MCP server logic. Depends only on `tokred-core` |
| `tokred` (`crates/tokred-cli`) | The binary. Parses arguments, builds the real adapters, calls the use cases, formats output, decides the exit code. The only crate that uses `anyhow` and `println!` |

The four proxy crates follow the same direction as the rest: `tokred-routing`, `tokred-suggest` and `tokred-compress` are pure and depend only on `tokred-core` (they plug into the proxy through the hook pipeline defined there); `tokred-proxy` is the adapter that runs them over HTTP; the binary wires everything and runs it with `tokred proxy start --foreground`. See [proxy](proxy.md).

Inside the binary, terminal I/O goes through a small `Io` seam (interactive flag, input, output) and command output is rendered by pure functions, so prompts and reports are unit-tested without a TTY.

The binary package is named `tokred` so `cargo install tokred` works; its directory keeps the `tokred-cli` name.

## Rules

- Use cases depend only on `tokred-core`. The core logic is **pure functions** over data (`merge_settings`, `remove_tokred`, `plan_agent`, `merge_config`…); I/O enters through traits passed as parameters.
- No use case imports `rusqlite`, `ureq` or calls `std::fs` directly; adapters do.
- Errors are `thiserror` enums per crate; `anyhow` only in the binary.
- Lints: `unsafe_code = forbid`; clippy `pedantic` as warning; `unwrap`, `expect`, `panic`, `todo`, `dbg!` denied outside tests.
- `Clock` exists so TTLs, the 24 h version check and per-day metrics are testable without waiting.

## Flow of a hook

1. Claude Code runs `"<path>/tokred" hook <event>` and writes JSON to stdin.
2. `main` handles `hook` **before** argument parsing, so even an unknown event exits 0.
3. The handler (`tokred-hooks`) runs inside `catch_unwind`; on error or panic it logs to `<data>/logs/` and produces no output.
4. If there is a decision, it is printed as JSON on stdout (`hookSpecificOutput`, `permissionDecision`, `additionalContext`, `systemMessage`); the process exits 0.

## Flow of `configure`

`select Claude folder → find stable binary path → plan_configure (read-only) → show plan → confirm → create data folder/config/database → apply_configure (backup, atomic write of settings.json, agents, manifest)`.
