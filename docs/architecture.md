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
| `tokred-hooks` | Hook handlers (fail-open stubs in F0) |
| `tokred-metrics`, `tokred-pricing`, `tokred-audit`, `tokred-update`, `tokred-report` | Use cases of later phases (placeholders in F0) |
| `tokred-store` | Adapter: SQLite (`rusqlite`), versioned migrations, repository implementations |
| `tokred` (`crates/tokred-cli`) | The binary. Parses arguments, builds the real adapters, calls the use cases, formats output, decides the exit code. The only crate that uses `anyhow` and `println!` |

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
