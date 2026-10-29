**English** | [Português](commands.pt-BR.md)

# Commands

Every command that prints a report accepts `--json` for machine-readable output. Table output is colored when stdout is a terminal; set `NO_COLOR=1` to turn colors off (`CLICOLOR_FORCE=1` forces them on).

| Command | What it does |
|---|---|
| `tokred configure` | Integrate with Claude Code |
| `tokred update` | Update according to how tokred was installed |
| `tokred unconfigure` | Remove tokred's parts from Claude Code |
| `tokred doctor` | Show what is installed and what needs attention |
| `tokred stats` | Tokens, cost, savings and projection, in the terminal |
| `tokred stats latency` | Time the hooks added to Claude Code |
| `tokred metrics` | The same numbers plus an HTML report |
| `tokred audit` | Weight of the fixed context and unused MCP servers |
| `tokred pricing sync` / `show` | Refresh or show the price table |
| `tokred cache stats` / `prune` | State of the tiered cache; remove expired entries |
| `tokred proxy start` / `stop` / `status` | Run and inspect the optional [local proxy](proxy.md) |
| `tokred proxy service install` / `uninstall` | Start the proxy at login, as a user service |
| `tokred claude [args...]` | Run Claude Code through the proxy, for this run only |
| `tokred routing status` / `on` / `off` / `explain` | [Model routing](routing.md) of subagent and auxiliary requests |
| `tokred compression on` / `off` / `status` | Reversible compression of large tool results |
| `tokred suggest` | Cheaper-model suggestions for the main loop, with the evidence |
| `tokred statusline` | One short line for Claude Code's status line |
| `tokred hook <event>` | Internal: called by Claude Code |
| `tokred mcp` | Internal: the `tokred_retrieve` MCP server, started by Claude Code (hidden) |

The folder of Claude Code can be given with `--claude-dir <path>` on every command that needs it.

## `tokred configure [--claude-dir <path>] [--yes] [--force]`

Integrates tokred with Claude Code.

1. Finds the Claude Code folder, in this order: `--claude-dir`, the value saved in `tokred.toml` (`[claude] dir`), `$CLAUDE_CONFIG_DIR`, `~/.claude` (`%USERPROFILE%\.claude` on Windows). A folder is valid when it exists and contains `settings.json` or `projects/`. An explicit `--claude-dir` that is not valid is an error. If nothing is found, an interactive terminal is asked for the path; otherwise the command fails and suggests `--claude-dir`.
2. Detects rtk and tells you the two will coexist.
3. Prints the **plan**: hooks to add (`+`), update (`~`) or keep (`=`), agents to create, update or leave alone, and the backup it will make. It asks for confirmation; `--yes` skips the question (required when stdin is not a terminal).
4. Applies it: creates the data folder, database and `tokred.toml`, merges `settings.json` (backup in `settings.json.tokred.bak`, only the last one is kept), and installs the agents.

Running it again is safe: the result is the same. If `settings.json` is not valid JSON the command aborts and **writes nothing**. `--force` overwrites agent files even if you edited them (otherwise your version is kept and the new one is written to `tokred-x.md.new`).

## `tokred update [--claude-dir <path>]`

Checks the latest release and acts according to how tokred was installed (detected from the path of the executable):

| Installed with | `tokred update` |
|---|---|
| Homebrew, winget, scoop, cargo | Prints the command to run (`brew upgrade tokred`, `winget upgrade tokred`, `scoop update tokred`, `cargo install tokred --force`) |
| Install script | Downloads the archive for this platform, **verifies its SHA-256**, replaces the binary atomically and re-applies the configuration with the new binary |

After the binary changes version, the first CLI command you run re-applies the configuration quietly (hooks never do this), so a package-manager upgrade needs no extra step. Needs the network; refuses to run when `TOKRED_OFFLINE` is set.

## `tokred unconfigure [--claude-dir <path>] [--purge]`

Removes the tokred hooks from `settings.json` and the agent files tokred installed and you did not edit. Edited agents are kept and reported. Files without the `tokred-` prefix are never touched, and anything in `settings.json` that tokred does not understand is left as it is. `--purge` also deletes the data folder; it refuses to do so if the folder does not contain `tokred.db` or `tokred.toml`.

## `tokred doctor [--claude-dir <path>] [--json] [--bench]`

Shows the tokred version, binary path, Claude Code and data folders, database schema version, config status, rtk detection, a table of hooks (`ok`, `missing`, `outdated`, or a binary path that no longer exists), a table of agents (`current`, `outdated`, `edited by you`, `missing`, `.new` pending), and a list of problems and notes. A proxy section shows whether the [proxy](proxy.md) is running, its routing and compression modes, and whether `ANTHROPIC_BASE_URL` points to it or to another gateway; when `HTTPS_PROXY` is set or extra CA files are configured, it also shows the proxy (host and port only, never the credentials) and the CA files. With `--bench` it also runs every hook many times with realistic input and measures the real latency from the outside (process start to exit); the result is saved and used by `stats latency`. `TOKRED_BENCH_RUNS` sets the runs per payload (default 50).

## `tokred stats [--since 7d] [--by model|day|project] [--json]`

The cost dashboard, printed in the terminal: totals (messages, tokens, real cost), what the same tokens would cost if everything ran on opus, the saving from routing and from the guards (**an estimate**, labelled as such), the prompt-cache hit rate, the monthly projection, an efficiency meter, and a numbered table with a share column and an impact bar. Models priced by family fallback are flagged. When data exists from before the first `configure`, the cost per session before and after is compared. Transcripts are ingested incrementally first, so it is fast after the first run. `--since` accepts hours, days and weeks (`12h`, `7d`, `2w`).

## `tokred stats latency [--since 7d] [--by hook|day|session] [--json]`

How much time the tokred hooks added to Claude Code: runs, p50/p95/p99/max per hook, total added time (an estimate: it includes the process start-up cost measured by `doctor --bench`), average per prompt, outcomes (ok, error, panic, timeout guard), an alert when a hook's p95 passes the 50 ms budget, and a comparison with what the guards saved in the same period.

## `tokred metrics [--since 30d] [--no-open]`

Prints the same dashboard as `stats` and writes the self-contained HTML report to `<data>/reports/metrics.html` (always the same file, overwritten atomically), then opens it in the browser unless `--no-open`. The report works offline: no network, no external fonts or scripts.

## `tokred audit [--project <path>] [--json]`

Estimates (characters / 4) the weight of the context Claude Code loads before you type: `CLAUDE.md` files with their `@` imports, rules, skills and agents; and lists MCP servers that were not used in the last 30 days. The project defaults to the current folder. The exact weight of MCP tool schemas cannot be computed locally, so only usage is shown.

## `tokred pricing sync` / `tokred pricing show [--json]`

`sync` refreshes the price table from the public LiteLLM list; if it cannot be reached the embedded snapshot stays in use. `show` prints the table (USD per million tokens). Prices are also synced automatically, at most once a day, by the commands above.

## `tokred cache stats [--json]` / `tokred cache prune`

`stats` shows entries and hits per tier (L1/L2/L3); `prune` removes expired entries. See [cache](cache.md).

## `tokred proxy start [--foreground] [--bind <host:port>]` / `stop` / `status [--json]`

The optional [local proxy](proxy.md). `start` runs it in the background (the same binary, `proxy start --foreground`); `--foreground` keeps it in the terminal, which is what the user service uses. `--bind` overrides `[proxy] bind` (default `127.0.0.1:8787`). `stop` asks it to shut down and waits until it has. `status` shows whether it is running, whether the user service is installed, whether this terminal's `ANTHROPIC_BASE_URL` points to it, and the routing and compression modes. With `--json` the `network` field has the HTTP proxy (host and port only) and the extra CA files the proxy uses. It is a background process that does not keep the output pipes of its caller, so `out=$(tokred proxy start)` returns (on Windows it is started through PowerShell's `Start-Process` for that reason).

## `tokred proxy service install` / `uninstall`

Installs the proxy as a user service that starts at login (a systemd user unit on Linux, a launchd agent on macOS, a logon task on Windows), or removes it. Nothing is installed unless you run this.

## `tokred claude [args...]`

Starts the proxy if it is not running and runs Claude Code through it, for this run only: nothing is written to Claude Code's settings. Every argument goes to `claude` unchanged and tokred exits with `claude`'s exit code. If the proxy cannot start, or `ANTHROPIC_BASE_URL` already points to another gateway, Claude Code runs as before and tokred says why on stderr.

## `tokred routing status [--json]` / `on` / `off` / `explain <id>`

`status` shows the mode, the circuit breaker and recent decisions. `on` sets `[routing] mode = "enforce"` and `off` sets `observe` (both edit `tokred.toml`; restart the proxy to apply). `explain <id>` says why one decision was taken. The main loop's model is never changed. See [routing](routing.md).

## `tokred compression on` / `off` / `status`

`on` registers the `tokred_retrieve` MCP server in Claude Code's user config (`~/.claude.json`, previous content saved as `.tokred.bak`) and sets `[compression] mode = "enforce"`: large tool results are replaced by a preview and the model reads the original back with `tokred_retrieve`. `off` unregisters the server and sets `observe` (measure only). `status` shows the mode, whether the server is registered and whether the circuit breaker tripped. Nothing is edited unless you run `on` or `off`.

## `tokred suggest [--since 7d] [--json]`

The suggestions for the main loop: how many prompts the proxy tracked, how many were simple, and the latest suggestions of a cheaper model. It only advises; switching is always your call (`/model`). Needs the proxy.

## `tokred statusline`

Reads Claude Code's status-line JSON on stdin and prints one short line (the pending suggestion and a few session figures). Set it as your status-line command if you want it.

## `tokred hook <event>` (internal)

Called by Claude Code, never by hand (except for testing, see [development](development.md)). Reads JSON on stdin, writes JSON on stdout. Events: `route`, `enforce`, `guard-read`, `guard-search`, `session-check`, `session-start`. It **always exits 0**, even for an unknown event or invalid input. See [hooks](hooks.md).

`tokred mcp` is also internal (hidden from `--help`): the MCP server with the `tokred_retrieve` tool that Claude Code starts when compression is on.
