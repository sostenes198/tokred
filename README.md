**English** | [Português](README.pt-BR.md)

# tokred

[![CI](https://github.com/sostenes198/tokred/actions/workflows/ci.yml/badge.svg)](https://github.com/sostenes198/tokred/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

*Token and cost reducer for Claude Code — model routing, context guards and local metrics. Works alongside rtk.*

## What it is

tokred is a local, cross-platform CLI written in Rust that lowers the cost and token usage of Claude Code. It installs subagents pinned to the right model for each kind of task (opus for architecture and planning, sonnet for development, haiku for search and summaries), hooks that reinforce that delegation and block useless context reads, and a metrics report with tokens, cost, savings and the latency tokred itself adds.

Everything runs on your machine, with its own SQLite database and no external service. It is designed to run next to [rtk](https://github.com/rtk-ai/rtk) (rust token killer), which compresses command output; tokred takes care of the rest. The two coexist: rtk owns the `Bash` matcher, tokred never registers one.

> **Status: phase F0.** `configure`, `unconfigure` and `doctor` work today. The hooks are installed but are fail-open stubs that do nothing yet; guards, routing, metrics and pricing arrive in later phases (see [the spec](docs/spec.md) and [the changelog](CHANGELOG.md)).

## Features

- **Non-destructive setup:** `tokred configure` merges its hooks into `settings.json` without touching anything else, and `tokred unconfigure` removes exactly what it added → [commands](docs/commands.md)
- **Model-routed subagents:** six subagents with a fixed model and a short return format → [agents](docs/agents.md)
- **Hooks that never get in the way:** fail-open, `off | warn | enforce` modes → [hooks](docs/hooks.md)
- **Local store:** SQLite with versioned migrations and a tiered cache → [cache](docs/cache.md)
- **Metrics** (planned): tokens, cost, savings, projection and added latency → [metrics](docs/metrics.md)

## Installation

Installing the binary never touches Claude Code; that is a separate step (`tokred configure`).

| OS | Command |
|---|---|
| macOS / Linux | `brew install sostenes198/tap/tokred` |
| Windows | `winget install tokred` or `scoop bucket add sostenes198 https://github.com/sostenes198/scoop-bucket` then `scoop install sostenes198/tokred` |
| Any | `cargo install tokred` or the shell / PowerShell installer from the releases page |

The release pipeline lands in phase F1; until then build from source (`cargo install --path crates/tokred-cli`). Details in [installation](docs/installation.md).

## Quick start

```sh
tokred configure    # integrate with Claude Code (shows a plan, asks to confirm)
tokred doctor       # check what is installed
```

`tokred metrics` joins the quick start in phase F1.

## Commands

| Command | What it does |
|---|---|
| `tokred configure [--claude-dir <path>] [--yes] [--force]` | Merge hooks into `settings.json`, install agents, create the database and config |
| `tokred unconfigure [--purge]` | Remove only tokred's parts; `--purge` also deletes the data folder |
| `tokred doctor [--json]` | Show what is installed and what needs attention |

Full reference in [commands](docs/commands.md).

## How it works

Claude Code runs a tokred hook before or after certain events and passes JSON on stdin; tokred answers with a decision or a one-line hint. Every hook is fail-open: on any error, panic or timeout it exits 0 with no output, so a bug in tokred can never block Claude Code. See [architecture](docs/architecture.md) and [hooks](docs/hooks.md).

## Configuration

A single `tokred.toml` in the data folder, merged (never replaced) on update. See [configuration](docs/configuration.md).

## Privacy

Everything stays on your machine. The only network calls (price sync and the daily version check, both from phase F1) are made from CLI commands, never from hooks, and can be turned off: `[pricing] auto_sync = false`, `[update] mode = "off"` or `TOKRED_NO_UPDATE_CHECK=1`.

## Uninstall

```sh
tokred unconfigure --purge   # remove hooks, agents and tokred's data
brew uninstall tokred        # or: winget uninstall tokred / scoop uninstall tokred / cargo uninstall tokred
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
