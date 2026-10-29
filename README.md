**English** | [Português](README.pt-BR.md)

# tokred

[![CI](https://github.com/sostenes198/tokred/actions/workflows/ci.yml/badge.svg)](https://github.com/sostenes198/tokred/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

*Token and cost reducer for Claude Code — model routing, context guards and local metrics. Works alongside rtk.*

## What it is

tokred is a local, cross-platform CLI written in Rust that lowers the cost and token usage of Claude Code. It installs subagents pinned to the right model for each kind of task (opus for architecture and planning, sonnet for development, haiku for search and summaries), hooks that reinforce that delegation and block useless context reads, and a metrics report with tokens, cost, savings and the latency tokred itself adds.

Everything runs on your machine, with its own SQLite database and no external service. It is designed to run next to [rtk](https://github.com/rtk-ai/rtk) (rust token killer), which compresses command output; tokred takes care of the rest. The two coexist: rtk owns the `Bash` matcher, tokred never registers one.

> **Status:** phases F0 to F5 are implemented: configure/unconfigure/doctor, the six hooks, metrics, pricing, audit, update and the release pipeline (see [the spec](docs/spec.md) and [the changelog](CHANGELOG.md)). An optional local proxy is also implemented, as an additive layer (measurement, routing of subagent and auxiliary requests, advice for the main loop, reversible compression): see [proxy](docs/proxy.md) and [routing](docs/routing.md). The [final spec](docs/spec.md) says what is done and what is still needed.

## Features

- **Non-destructive setup:** `tokred configure` merges its hooks into `settings.json` without touching anything else, and `tokred unconfigure` removes exactly what it added → [commands](docs/commands.md)
- **Model-routed subagents:** six subagents with a fixed model and a short return format → [agents](docs/agents.md)
- **Hooks that never get in the way:** fail-open, `off | warn | enforce` modes; they route work to the right model and block useless reads → [hooks](docs/hooks.md)
- **Numbers in your terminal:** `tokred stats` shows tokens, cost, savings, projection and the latency tokred adds, as a colored dashboard; `tokred metrics` also writes an offline HTML report → [metrics](docs/metrics.md)
- **Optional local proxy:** sits between Claude Code and the API to measure every request exactly, route subagent and auxiliary requests to a cheaper model, advise you about the main loop and keep large tool results out of the context without losing them. Off until you run `tokred claude`; the main loop's model is never changed → [proxy](docs/proxy.md), [routing](docs/routing.md)
- **Live dashboard:** `tokred dashboard` opens, in your browser, the sessions running right now and the last 10, with what was sent to Claude, what came back and the subagents running, in real time → [dashboard](docs/dashboard.md)
- **Audit:** how heavy your fixed context is and which MCP servers you never use → [commands](docs/commands.md)
- **Local store:** SQLite with versioned migrations and a tiered cache → [cache](docs/cache.md)

## Installation

Installing the binary never touches Claude Code; that is a separate step (`tokred configure`).

| OS | Command |
|---|---|
| macOS / Linux | `brew install sostenes198/tap/tokred` |
| Windows | `winget install tokred` or `scoop bucket add sostenes198 https://github.com/sostenes198/scoop-bucket` then `scoop install sostenes198/tokred` |
| Any | `cargo install tokred` or the shell / PowerShell installer from the releases page |

Until the first release is published, build from source (`cargo install --path crates/tokred-cli`). Details in [installation](docs/installation.md).

## Quick start

```sh
tokred configure    # integrate with Claude Code (shows a plan, asks to confirm)
tokred doctor       # check what is installed
tokred stats        # tokens, cost and savings, right in the terminal
tokred metrics      # the same, plus the HTML report
```

## Commands

| Command | What it does |
|---|---|
| `tokred configure [--claude-dir <path>] [--yes] [--force]` | Merge hooks into `settings.json`, install agents, create the database and config |
| `tokred unconfigure [--purge]` | Remove only tokred's parts; `--purge` also deletes the data folder |
| `tokred doctor [--json] [--bench]` | Show what is installed and what needs attention; measure hook latency |
| `tokred stats [--since 7d] [--by model\|day\|project]` | Dashboard of tokens, cost, savings and projection |
| `tokred stats latency` | Time the hooks added to Claude Code |
| `tokred metrics [--since 30d] [--no-open]` | The dashboard plus an HTML report |
| `tokred dashboard [--no-open]` | Open the proxy's live dashboard in the browser |
| `tokred audit [--project <path>]` | Weight of the fixed context and unused MCP servers |
| `tokred update` | Update according to how tokred was installed |
| `tokred pricing sync\|show`, `tokred cache stats\|prune` | Price table and cache |
| `tokred claude [args...]` | Run Claude Code through the optional proxy, for this run only |
| `tokred proxy start\|stop\|status`, `tokred proxy service install\|uninstall` | Run and inspect the proxy; start it at login |
| `tokred routing status\|on\|off\|explain`, `tokred compression on\|off\|status` | Model routing and reversible compression (both `observe` by default) |
| `tokred suggest`, `tokred statusline` | Cheaper-model suggestions for the main loop |

Full reference in [commands](docs/commands.md).

## How it works

Claude Code runs a tokred hook before or after certain events and passes JSON on stdin; tokred answers with a decision or a one-line hint. Every hook is fail-open: on any error, panic or timeout it exits 0 with no output, so a bug in tokred can never block Claude Code. See [architecture](docs/architecture.md) and [hooks](docs/hooks.md).

## Configuration

A single `tokred.toml` in the data folder, merged (never replaced) on update. See [configuration](docs/configuration.md).

## Privacy

Everything stays on your machine. The only network calls (the daily price sync and version check, and `update`) are made from CLI commands, never from hooks, and can be turned off: `[pricing] auto_sync = false`, `[update] mode = "off"`, `TOKRED_NO_UPDATE_CHECK=1`, or `TOKRED_OFFLINE=1` for all of them. The optional proxy listens on `127.0.0.1` only, never logs credentials, does not read or store message contents unless you turn on `[proxy] log_bodies` for the [dashboard](docs/dashboard.md) (off by default; [decision](docs/decisions.md)), and forwards requests only to the Anthropic API.

## Uninstall

```sh
tokred unconfigure --purge   # remove hooks, agents and tokred's data
brew uninstall tokred        # or: winget uninstall tokred / scoop uninstall tokred / cargo uninstall tokred
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). To run and test it locally, see [testing](docs/testing.md); to publish a release (Homebrew, Scoop, winget, crates.io), see [release](docs/release.md).

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
