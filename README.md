**English** | [Português](README.pt-BR.md)

# tokred

[![CI](https://github.com/sostenes198/tokred/actions/workflows/ci.yml/badge.svg)](https://github.com/sostenes198/tokred/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/sostenes198/tokred?include_prereleases)](https://github.com/sostenes198/tokred/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#contributing-security-license)

Lower the cost and token use of Claude Code: model routing, context guards and local metrics.

## What it is

tokred is a local command-line tool for macOS, Linux and Windows. It works in two layers:

1. **Hooks, subagents and metrics.** Six subagents pinned to the right model for each kind of task, hooks that reinforce that delegation and block useless context reads, and a report of tokens, cost and savings.
2. **An optional local proxy** between Claude Code and the Anthropic API. It measures every request exactly, advises you about the main loop (it never changes the main loop's model) and serves a live dashboard. Two more features start in a measure-only mode and change nothing until you turn them on: routing subagent and auxiliary requests to a cheaper model, and replacing large tool results with a preview you can read back in full.

Everything runs on your machine, with no account and no telemetry. Savings figures are estimates, not guarantees.

## Requirements

- [Claude Code](https://code.claude.com)
- macOS, Linux or Windows. You do not need Rust to use tokred.

## Install

**macOS and Linux**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/install.sh | sh
```

**Windows (PowerShell)**

```powershell
irm https://github.com/sostenes198/tokred/releases/latest/download/install.ps1 | iex
```

The installer:

- downloads the release for your platform and verifies its SHA-256 checksum;
- installs the binary in `~/.tokred/bin` (Windows: `%LOCALAPPDATA%\tokred\bin`) and adds it to your `PATH`, with no `sudo`;
- runs `tokred configure` (hooks and agents) and `tokred proxy enable` (asks for the proxy port and whether the dashboard may record conversation content; `--no-proxy` skips it).

It also defines a `claude` wrapper in your shell profile that keeps Claude Code's settings in line with the proxy switch before each launch; `command claude` runs the real binary.

Pre-releases are installed only when you ask for their version. To pick a version, and for every other option, see [installation](docs/installation.md).

## Quick start

Check what is installed:

```sh
tokred doctor
```

See tokens, cost and savings in the terminal:

```sh
tokred stats
```

Run Claude Code through the proxy, for this run only:

```sh
tokred claude
```

Open the live dashboard of your sessions (needs the proxy):

```sh
tokred dashboard
```

## Commands

| Command | What it does |
|---|---|
| `tokred configure` | Add tokred's hooks and agents to Claude Code, create the database and config |
| `tokred doctor` | Show what is installed and what needs attention |
| `tokred stats` | Tokens, cost, savings and projection |
| `tokred metrics` | The same, plus an HTML report |
| `tokred dashboard` | Open the live dashboard in your browser |
| `tokred proxy enable` / `disable` / `status` | Turn the proxy on or off, or see what it is doing |
| `tokred update` | Update tokred (when the script installed it) |
| `tokred unconfigure` | Remove only tokred's hooks and agents |
| `tokred uninstall` | Remove tokred completely |

Every command and option: [commands](docs/commands.md).

## How it works

Claude Code runs a tokred hook before or after certain events and passes JSON on stdin; tokred answers with a decision or a one-line hint. Hooks are fail-open: on any error, panic or timeout they exit 0 with no output, so a bug in tokred cannot block Claude Code. The proxy, when enabled, forwards each request to the Anthropic API and records what passed through. See [architecture](docs/architecture.md).

## Privacy

- **Local.** Metrics and settings live in your own data folder. The proxy listens on `127.0.0.1` only, never logs credentials, and forwards requests only to the Anthropic API.
- **Message contents** are stored only if you turn on `[proxy] log_bodies` (the installer asks; the default without a terminal is off). Details: [dashboard](docs/dashboard.md) and [proxy](docs/proxy.md).
- **Network.** Apart from the proxy, tokred contacts GitHub (at most one version check a day, and `tokred update`) and downloads a public price list (at most once a day, and `tokred pricing sync`). Hooks never use the network. Switch the automatic calls off with `[update] mode = "off"`, `[pricing] auto_sync = false`, or `TOKRED_OFFLINE=1` for both. See [configuration](docs/configuration.md).

## Documentation

The full index is in [docs/README.md](docs/README.md): installation, commands, configuration, hooks and agents, metrics, proxy, routing, dashboard, and the contributor guides.

## Uninstall

**macOS and Linux**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/uninstall.sh | sh
```

**Windows (PowerShell)**

```powershell
irm https://github.com/sostenes198/tokred/releases/latest/download/uninstall.ps1 | iex
```

It removes everything without asking: the proxy and its service, the hooks and agents, tokred's `ANTHROPIC_BASE_URL` in Claude Code's settings, tokred's data folder, the binary, the `PATH` entry and the `claude` wrapper.

## Contributing, security, license

- Contributions are welcome: see [CONTRIBUTING.md](CONTRIBUTING.md) and the [code of conduct](CODE_OF_CONDUCT.md).
- Report vulnerabilities privately: see [SECURITY.md](SECURITY.md).
- Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
