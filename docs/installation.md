**English** | [Português](installation.pt-BR.md)

# Installation

How to install, update and remove tokred, and what the installer changes on your machine.

## Install

**macOS and Linux**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/install.sh | sh
```

**Windows (PowerShell)**

```powershell
irm https://github.com/sostenes198/tokred/releases/latest/download/install.ps1 | iex
```

The scripts are release assets and live in [`install/`](../install/); they are short, so read them before you run them. No `sudo` and no administrator rights are needed.

Until the first stable release is published, the `latest` URLs do not exist. Use the scripts from `main` instead (`https://raw.githubusercontent.com/sostenes198/tokred/main/install/install.sh` and `https://raw.githubusercontent.com/sostenes198/tokred/main/install/install.ps1`) together with `--version`, or build from source. Pre-releases are installed only when you ask for their version explicitly.

## What the installer does

1. Detects your platform: macOS or Linux (musl, so it runs on every distribution) on x86_64 or aarch64, or Windows on x86_64 (Windows on ARM runs that build through emulation).
2. Downloads `tokred-<target>.tar.gz` (Windows: `.zip`) and its `.sha256` from the GitHub release and **verifies the checksum**. A mismatch stops everything before anything is written.
3. Installs the binary in `${TOKRED_HOME:-$HOME/.tokred}/bin` (Windows: `%LOCALAPPDATA%\tokred\bin`).
4. Adds that folder to your `PATH` and defines the [`claude` wrapper](#the-claude-wrapper) in the shell profiles that exist (`~/.bashrc`, `~/.bash_profile`, `~/.zshrc`, `~/.profile` and, when fish is installed, `~/.config/fish/conf.d/tokred.fish`). On Windows it uses your user `PATH` and your PowerShell profile. Everything it writes sits inside one marked block (`# >>> tokred >>>` ... `# <<< tokred <<<`) that is replaced on every run, so running the script twice changes nothing.
5. Runs `tokred configure`, which adds tokred's hooks and agents to Claude Code (see [hooks and agents](hooks-and-agents.md)).
6. Runs `tokred proxy enable`, which asks the two questions below, saves the answers, starts the [proxy](proxy.md) and points Claude Code at it.

The script never writes outside the tokred folder and your profile files, and refuses to run as root unless you pass `--yes`. With `curl ... | sh` it asks its questions on the terminal (`/dev/tty`); with no terminal it applies the defaults.

**Which port does the proxy use?** Default `1029`, or the one already in `tokred.toml`. Only 1024 to 65535 is accepted; the installer checks that the port is free and asks again when it is busy.

**Record conversation content so the dashboard can show it?** The text stays in tokred's own database on your machine; headers and credentials are never recorded. The answer is saved as `[proxy] log_bodies`. Without a terminal, or with `--yes`, content is not recorded unless you say so. Change it later by editing `tokred.toml` or running `tokred proxy enable` again.

## Options

`irm | iex` cannot pass parameters, so every option also has an environment variable. The same variables work for `sh`.

| Shell option | PowerShell | Environment variable | Meaning |
|---|---|---|---|
| `-y`, `--yes` | `-Yes` | `TOKRED_YES=1` | Ask nothing; use the defaults |
| `--port N` | `-Port N` | `TOKRED_PORT` | Proxy port (1024 to 65535) |
| `--version X` | `-Version X` | `TOKRED_VERSION` | Install release `vX` instead of the latest |
| `--no-modify-path` | `-NoModifyPath` | `TOKRED_NO_MODIFY_PATH=1` | Leave `PATH` alone |
| `--no-alias` | `-NoAlias` | `TOKRED_NO_ALIAS=1` | Do not define the `claude` wrapper |
| `--no-proxy` | `-NoProxy` | `TOKRED_NO_PROXY=1` | Hooks and agents only; do not run `tokred proxy enable` |
| `--no-configure` | `-NoConfigure` | `TOKRED_NO_CONFIGURE=1` | Only install the binary (no `configure`, no proxy) |
| | | `TOKRED_HOME` | Install folder (default `~/.tokred`) |
| | | `TOKRED_RELEASE_BASE_URL` | Where release assets are downloaded from (`https://` or `file://`), for mirrors and offline installs |

`--no-modify-path` and `--no-alias` are independent; with both, your profiles are not touched at all. Examples:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/install.sh | sh -s -- --yes --port 2000 --no-alias
```

```powershell
$env:TOKRED_PORT = '2000'; $env:TOKRED_NO_ALIAS = '1'; irm https://github.com/sostenes198/tokred/releases/latest/download/install.ps1 | iex
```

For an offline install, point `TOKRED_RELEASE_BASE_URL` at a folder (or URL) that holds `tokred-<target>.tar.gz` / `.zip` and its `.sha256` file; the checksum is always verified.

## The `claude` wrapper

The installer adds a shell function named `claude` that runs `tokred initialize` and then the real Claude Code, with your arguments untouched:

```sh
claude() { if command -v tokred >/dev/null 2>&1; then tokred initialize; fi; command claude "$@"; }
```

`tokred initialize` is quiet and fast, and always exits 0. It makes Claude Code's settings agree with the proxy switch (`[proxy] enabled`): when the proxy is on it points Claude Code at it (and starts it if needed); when it is off it removes only tokred's own `ANTHROPIC_BASE_URL` and leaves a gateway of yours alone.

| You want to | Do this |
|---|---|
| Run the real `claude` once | `command claude` (PowerShell: `& (Get-Command claude -CommandType Application)`) |
| Turn the proxy off or on | `tokred proxy disable` / `tokred proxy enable` |
| Not install the wrapper | `--no-alias`, `-NoAlias` or `TOKRED_NO_ALIAS=1` |
| Remove the wrapper later | Delete the block between the `# >>> tokred >>>` markers, or [uninstall](#uninstall) |

The wrapper is fail-open: if `tokred` is missing, `claude` starts as usual. It covers bash, zsh, fish and PowerShell; `cmd.exe` has no persistent aliases, so start Claude Code from PowerShell or run `tokred initialize` by hand first. Claude Code's own installer may add `alias claude=...` to `~/.zshrc`; the block runs `unalias claude` first, so put the real `claude` on your `PATH` if that alias was the only way to reach it.

## Update

```sh
tokred update
```

```sh
tokred update --version 0.2.0
```

`tokred update` looks at where the running binary lives. In the install folder (put there by the script) it downloads the archive for your platform, verifies its SHA-256, replaces the binary and re-applies the configuration. Anywhere else (built from source, copied by hand) it says a new version exists and asks you to reinstall with the script, which does the same job. Without `--version` it installs the latest stable release. After the binary changes version, the first CLI command re-applies the configuration; your port and content-recording choices are kept. `tokred doctor` shows when that is pending.

## Uninstall

**macOS and Linux**

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/uninstall.sh | sh
```

**Windows (PowerShell)**

```powershell
irm https://github.com/sostenes198/tokred/releases/latest/download/uninstall.ps1 | iex
```

From a clone, run `sh install/uninstall.sh`. It removes everything **without asking** by running `tokred uninstall`, which:

1. stops the proxy and removes its user service, if installed;
2. turns compression off, if it was on;
3. removes tokred's `ANTHROPIC_BASE_URL` from Claude Code's settings (a gateway of your own stays);
4. removes tokred's hooks and agents from Claude Code (agents you edited are kept and reported);
5. deletes tokred's data folder (database, config, logs, reports);
6. removes the binary and install folder, the `PATH` entry and the `claude` wrapper blocks in your shell profiles (a fish file that holds only tokred's block is deleted). On Windows this last part runs as soon as `tokred.exe` has exited, because a running program cannot delete itself.

Every step is best effort: a failure is reported and the next step still runs, and if any step failed the binary is kept so you can run the command again. `tokred uninstall` can also be run by itself. Without the binary, the script cleans what is left in your shell profiles.

## Build from source

You need Rust 1.89 or newer.

```sh
cargo install --path crates/tokred-cli
```

A binary built this way works but does not update itself: `tokred update` tells you to reinstall. To work on tokred, see [development](development.md).

## Troubleshooting

| Problem | What to do |
|---|---|
| `tokred: command not found` after installing | Open a new terminal so the new `PATH` is loaded, or check that `~/.tokred/bin` (Windows: `%LOCALAPPDATA%\tokred\bin`) is on it. Without `--no-modify-path` the installer adds it. |
| The `claude` wrapper is missing in new PowerShell windows | Your execution policy may be `Restricted`. Run `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` once. |
| The proxy port is busy | Pick another one: `tokred proxy enable --port 2000`, or install with `--port 2000`. |
| The proxy cannot reach the API from a corporate network | The proxy honours `HTTPS_PROXY`, `NO_PROXY` and friends; see [proxy](proxy.md). |
| Something else looks wrong | Run `tokred doctor`; it shows what is installed and what needs attention. |
