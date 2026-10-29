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

Like rustup: one command downloads tokred from the GitHub release, checks its checksum, installs it in `~/.tokred/bin`, adds that folder to your `PATH` and runs `tokred configure` (hooks and agents) and then `tokred proxy enable`, which asks which port the proxy uses (default 1029) and whether the dashboard may record conversation content (`--no-proxy` skips it). No sudo, no package manager.

```sh
# macOS and Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/install.sh | sh

# Windows (PowerShell)
irm https://github.com/sostenes198/tokred/releases/latest/download/install.ps1 | iex
```

It also defines a `claude` wrapper in your shell profile (bash, zsh, fish, PowerShell): `claude` runs `tokred initialize` first, then the real Claude Code. `tokred proxy disable` / `enable` is the switch, `command claude` always runs the real binary, and `--no-alias` skips the wrapper. Options (`--yes`, `--port`, `--version`, `--no-modify-path`, `--no-alias`, `--no-configure`), the uninstall command and the details are in [installation](docs/installation.md). Before the first release exists, the same scripts are served from the `main` branch: `https://raw.githubusercontent.com/sostenes198/tokred/main/install/install.sh` (and `install.ps1`).

## Quick start

```sh
tokred configure    # integrate with Claude Code (the installer already ran it; run it again any time)
tokred doctor       # check what is installed
tokred stats        # tokens, cost and savings, right in the terminal
tokred metrics      # the same, plus the HTML report
```

## Commands

| Command | What it does |
|---|---|
| `tokred configure [--claude-dir <path>] [--yes] [--force]` | Merge hooks into `settings.json`, install agents, create the database and config (nothing about the proxy: that is `proxy enable`) |
| `tokred unconfigure [--purge]` | Remove only tokred's parts from Claude Code; `--purge` also deletes the data folder |
| `tokred uninstall [--yes] [--purge]` | Stop the proxy and its service, remove hooks and agents; the uninstall script then removes the binary |
| `tokred doctor [--json] [--bench]` | Show what is installed and what needs attention; measure hook latency |
| `tokred stats [--since 7d] [--by model\|day\|project]` | Dashboard of tokens, cost, savings and projection |
| `tokred stats latency` | Time the hooks added to Claude Code |
| `tokred metrics [--since 30d] [--no-open]` | The dashboard plus an HTML report |
| `tokred dashboard [--no-open]` | Open the proxy's live dashboard in the browser |
| `tokred audit [--project <path>]` | Weight of the fixed context and unused MCP servers |
| `tokred update` | Update itself when it was installed by the script; otherwise say to reinstall |
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
# macOS and Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/uninstall.sh | sh

# Windows (PowerShell)
irm https://github.com/sostenes198/tokred/releases/latest/download/uninstall.ps1 | iex
```

It runs `tokred uninstall` (stops the proxy, removes its user service, takes the hooks and agents out of Claude Code, asks whether to delete tokred's data), then removes the binary, the `PATH` entry and the `claude` wrapper.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). To run and test it locally, see [testing](docs/testing.md); to publish a release (a tag; the install scripts and archives are GitHub release assets), see [release](docs/release.md).

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
