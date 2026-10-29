**English** | [Português](installation.pt-BR.md)

# Installation

tokred installs like rustup: one command in the terminal downloads the binary from the GitHub release, checks its checksum, puts it in your home folder, wires your shell and runs `tokred configure` and `tokred proxy enable`. There is no package manager involved (no Homebrew, Scoop, winget or crates.io), no `sudo` and no administrator rights.

```sh
# macOS and Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/install.sh | sh

# Windows (PowerShell)
irm https://github.com/sostenes198/tokred/releases/latest/download/install.ps1 | iex
```

The scripts are release assets (`install.sh`, `install.ps1`, `uninstall.sh`, `uninstall.ps1`). Before the first release exists, or to try the version on `main`, use the raw URL instead: `https://raw.githubusercontent.com/sostenes198/tokred/main/install/install.sh` (and `install.ps1`). The scripts live in [`install/`](../install/); read them before you run them, they are short.

## What the script does

1. Detects your platform: macOS or Linux (musl, so it runs on every distribution) on x86_64 or aarch64; Windows on x86_64 or aarch64.
2. Downloads `tokred-<target>.tar.gz` (Windows: `.zip`) and its `.sha256` from the release, and **verifies the checksum**. A mismatch stops everything before anything is written.
3. Installs the binary in `${TOKRED_HOME:-$HOME/.tokred}/bin` (Windows: `%LOCALAPPDATA%\tokred\bin`).
4. Adds that folder to your `PATH` and defines the [`claude` wrapper](#the-claude-wrapper): in the shell profiles that exist (`~/.bashrc`, `~/.bash_profile`, `~/.zshrc`, plus `~/.profile` and, when fish is installed, `~/.config/fish/conf.d/tokred.fish`); on Windows in your user `PATH` and your PowerShell profile. Everything it writes sits inside one marked block (`# >>> tokred >>>` ... `# <<< tokred <<<`), which is replaced on every run, so running the script twice changes nothing, and removed by the uninstall script.
5. Runs [`tokred configure`](commands.md), which hooks tokred into Claude Code (hooks and agents only; it asks nothing about the proxy).
6. Runs [`tokred proxy enable`](commands.md#tokred-proxy-enable---port-n---record-content-truefalse---yes--disable), which asks the two questions below, saves the answers, starts the [proxy](proxy.md) and points Claude Code at it. `--no-proxy` skips this step, so you get the hooks and agents only (you can run `tokred proxy enable` any time later).

It never uses `sudo`, never writes outside the tokred folder and your profile files, and refuses to run as root unless you pass `--yes`. With `curl ... | sh` the script's stdin is the pipe, so questions are asked on the terminal (`/dev/tty`); with no terminal at all it applies the defaults.

### The two questions

They are `tokred proxy enable`'s, asked after `configure` has finished.

- **Which port does the proxy use?** Default `1029` (or the one already in `tokred.toml`). Only 1024 to 65535 is accepted; the installer checks that the port is free by trying to bind `127.0.0.1:<port>`, says so when it is busy and asks again. If the proxy of this installation already listens on it, that is fine.
- **Record conversation content so the dashboard can show what was sent and returned? [Y/n]** The text stays in tokred's own database on your machine; headers and credentials are never recorded. The answer is saved as `[proxy] log_bodies` in `tokred.toml`. Without a terminal, or with `--yes`, content is **not** recorded unless you say so (`tokred proxy enable --record-content true`, `TOKRED_RECORD_CONTENT=1`). Change it later by editing `tokred.toml` or running `tokred proxy enable` again.

### Options

`irm | iex` cannot pass parameters, so every option also has an environment variable (the same variables work for `sh`).

| Shell option | PowerShell | Environment variable | Meaning |
|---|---|---|---|
| `-y`, `--yes` | `-Yes` | `TOKRED_YES=1` | Ask nothing; use the defaults and the options below |
| `--port N` | `-Port N` | `TOKRED_PORT` | Proxy port (1024 to 65535), passed to `tokred proxy enable --port` |
| `--version X` | `-Version X` | `TOKRED_VERSION` | Install release `vX` instead of the latest |
| `--no-modify-path` | `-NoModifyPath` | `TOKRED_NO_MODIFY_PATH=1` | Leave `PATH` alone |
| `--no-alias` | `-NoAlias` | `TOKRED_NO_ALIAS=1` | Do not define the `claude` wrapper |
| `--no-proxy` | `-NoProxy` | `TOKRED_NO_PROXY=1` | Hooks and agents only: do not run `tokred proxy enable` |
| `--no-configure` | `-NoConfigure` | `TOKRED_NO_CONFIGURE=1` | Only install the binary (no `configure`, no proxy) |
| | | `TOKRED_HOME` | Install folder (default `~/.tokred`, Windows `%LOCALAPPDATA%\tokred`) |
| | | `TOKRED_RELEASE_BASE_URL` | Where the release assets are downloaded from (`https://` or `file://`); for mirrors and offline installs |

`--no-modify-path` and `--no-alias` are independent: the first skips `PATH`, the second skips the wrapper; with both, the profiles are not touched at all. Examples:

```sh
curl --proto '=https' --tlsv1.2 -LsSf .../install.sh | sh -s -- --yes --port 2000 --no-alias
curl --proto '=https' --tlsv1.2 -LsSf .../install.sh | sh -s -- --no-proxy    # hooks and agents only
TOKRED_VERSION=0.2.0 TOKRED_NO_ALIAS=1 sh install.sh
```

```powershell
$env:TOKRED_PORT = '2000'; $env:TOKRED_NO_ALIAS = '1'; irm .../install.ps1 | iex
powershell -File install.ps1 -Yes -Port 2000          # or run the downloaded file with parameters
```

## The `claude` wrapper

The installer adds a shell function named `claude` that runs `tokred initialize` and then the real Claude Code, with your arguments untouched:

```sh
claude() { if command -v tokred >/dev/null 2>&1; then tokred initialize; fi; command claude "$@"; }
```

`tokred initialize` is quiet and fast, and it always exits 0: it makes Claude Code's settings agree with the proxy switch (`[proxy] enabled`): when the proxy is on it points Claude Code at it (and starts it if needed); when it is off it removes only tokred's own `ANTHROPIC_BASE_URL` and leaves another gateway of yours alone. So `claude` always does what the switch says, in every terminal.

- **It is fail-open.** If `tokred` is missing (uninstalled, moved), `claude` starts as usual.
- **Bypass it:** `command claude` runs the real binary without the wrapper (PowerShell: `& (Get-Command claude -CommandType Application)`).
- **Switch the proxy off or on:** `tokred proxy disable` / `tokred proxy enable`.
- **Do not install it:** `--no-alias` / `-NoAlias` / `TOKRED_NO_ALIAS=1`. To remove it later, delete the block between the `# >>> tokred >>>` markers or run the uninstall script.
- **Covered shells:** bash, zsh, fish and PowerShell (Windows PowerShell and pwsh). `cmd.exe` has no persistent aliases, so it is not wrapped; start Claude Code from PowerShell, or run `tokred initialize` by hand first.
- **A function, not an alias,** so it passes arguments through and cannot call itself. The block runs `unalias claude` first because Claude Code's own installer may add `alias claude=...` to `~/.zshrc`; if that alias was the only way to reach `claude`, `command claude` will not find it. Put the real `claude` on your `PATH` instead.
- **PowerShell** needs a profile that loads: if new windows do not get the wrapper, your execution policy may be `Restricted`; run `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` once.

## Install a specific version, offline or from a mirror

```sh
sh install.sh --version 0.2.0
TOKRED_RELEASE_BASE_URL=https://mirror.example/tokred/v0.2.0 sh install.sh
TOKRED_RELEASE_BASE_URL=file:///srv/tokred sh install.sh      # a folder with the archive and its .sha256
```

The folder (or URL) must hold `tokred-<target>.tar.gz` / `.zip` and `tokred-<target>.tar.gz.sha256` / `.zip.sha256`; the checksum is always verified. To check a download by hand: `shasum -a 256 tokred-*.tar.gz` (Windows: `Get-FileHash`) and compare with the `.sha256` file.

## Building from source

For development: `cargo build` and run `target/debug/tokred`, see [testing](testing.md). A binary built this way (or placed anywhere outside the install folder) works but does not update itself: `tokred update` tells you to reinstall with the script.

## Optional: the local proxy

The [proxy](proxy.md) is controlled by one switch, `[proxy] enabled` in `tokred.toml` (`tokred proxy enable` turns it on and the install script runs it, unless you pass `--no-proxy`; `tokred proxy disable` turns it off; `tokred configure` never touches it). While it is on, `tokred initialize` (what the `claude` shell wrapper runs before every launch) starts the proxy when needed and keeps `env.ANTHROPIC_BASE_URL` in Claude Code's `settings.json` pointing at it; `tokred unconfigure` removes that URL again. Your own gateway in `ANTHROPIC_BASE_URL` is never overwritten.

```sh
tokred claude                     # start the proxy if needed and run Claude Code through it, for this run only
tokred proxy service install      # optional: start the proxy at login, as a user service
```

`tokred compression on` is the only command that edits Claude Code's user config (`~/.claude.json`, with a `.tokred.bak` backup), and only when you run it.

## Updating

`tokred update` looks at where the running binary lives:

| Where the binary is | What `tokred update` does |
|---|---|
| In the install folder (`$TOKRED_HOME/bin`), put there by the script | Downloads the archive for this platform from the GitHub release, **verifies its SHA-256**, replaces the binary atomically (on Windows the running `.exe` is renamed to `.exe.old` and removed on the next start) and re-applies the configuration with the new binary |
| Anywhere else (built from source, copied by hand) | Says a new version exists and to reinstall with the install script |

Running the install script again does the same job and also works for a binary that cannot update itself. After the binary changes version, the first CLI command re-applies the configuration non-interactively (hooks never do this); your port and content-recording choices are kept. `tokred doctor` shows when a reconfiguration is pending.

## Version check

Once a day, from CLI commands only (never from a hook), tokred asks GitHub for the latest release with a 2 s timeout and prints a one-line notice on **stderr** if a newer version exists, so `--json` output is not affected. Disable it with `[update] mode = "off"` or `TOKRED_NO_UPDATE_CHECK=1`; `TOKRED_OFFLINE=1` turns off every network call (version check and price sync).

## Uninstalling

```sh
# macOS and Linux
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/uninstall.sh | sh

# Windows (PowerShell)
irm https://github.com/sostenes198/tokred/releases/latest/download/uninstall.ps1 | iex
```

(Or `sh install/uninstall.sh` from a clone.) Options: `-y/--yes` (`-Yes`, `TOKRED_YES=1`) asks nothing and keeps your data; `--purge` (`-Purge`, `TOKRED_PURGE=1`) also deletes tokred's data folder. Without them it asks.

The script first runs `tokred uninstall`, which:

1. stops the proxy and removes its user service, if installed;
2. turns compression off, if it was on;
3. removes tokred's hooks and agents from Claude Code (`tokred unconfigure`; agents you edited are kept);
4. asks whether to delete tokred's data folder (database, config, logs, reports), unless you passed `--yes` or `--purge`.

Every step is best effort: a failure is reported and the next step still runs. If you answer "no" to the confirmation, nothing is removed. Then the script removes the binary (on Windows only after `tokred.exe` has exited: a running program cannot delete itself), the install folder if it is empty, the `PATH` entry and the `claude` wrapper blocks (a fish file that holds only our block is deleted; one with other lines keeps them). `tokred uninstall` can also be run by itself; it then prints what is left for the script.

## Publishing a release (maintainers)

Push a tag `vX.Y.Z`; the generated `release.yml` builds the archives and uploads them, their `.sha256` files and the four install scripts to the GitHub release. No tokens, tap or bucket repositories are needed. Step by step: [release](release.md).
