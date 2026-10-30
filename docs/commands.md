**English** | [Português](commands.pt-BR.md)

# Commands

Reference for every `tokred` command and its flags. `tokred <command> --help` prints the same flags for your version.

| Command | What it does |
|---|---|
| [`configure`](#configure) | Add tokred's hooks and agents to Claude Code |
| [`unconfigure`](#unconfigure) | Remove tokred's hooks and agents from Claude Code |
| [`update`](#update) | Replace the binary with a newer (or a specific) release |
| [`uninstall`](#uninstall) | Remove tokred from the machine completely |
| [`doctor`](#doctor) | Show what is installed and what needs attention |
| [`stats`](#stats) | Tokens, cost, savings and projection in the terminal |
| [`stats latency`](#stats-latency) | Time the hooks added to Claude Code |
| [`metrics`](#metrics) | The same numbers plus an HTML report |
| [`audit`](#audit) | Weight of the fixed context and unused MCP servers |
| [`pricing sync` / `show`](#pricing) | Refresh or show the price table |
| [`cache stats` / `prune`](#cache) | Inspect the tiered cache or remove expired entries |
| [`proxy start` / `stop` / `restart` / `status`](#proxy-start-stop-restart-status) | Run and inspect the optional [local proxy](proxy.md) |
| [`proxy enable` / `disable`](#proxy-enable-disable) | Turn the proxy features on or off and apply it at once |
| [`proxy service install` / `uninstall`](#proxy-service) | Start the proxy at login, as a user service |
| [`initialize`](#initialize) | Make Claude Code's settings agree with `[proxy] enabled` |
| [`claude`](#claude) | Run Claude Code through the proxy, for this run only |
| [`dashboard`](#dashboard) | Open the proxy's [live dashboard](dashboard.md) |
| [`routing status` / `on` / `off` / `explain`](#routing) | [Model routing](routing.md) of subagent and auxiliary requests |
| [`compression on` / `off` / `status`](#compression) | Reversible compression of large tool results |
| [`model-guard on` / `warn` / `off` / `limit` / `status`](#model-guard) | What a model switch costs in prompt cache: tell you, or ask you to confirm |
| [`suggest`](#suggest) | Cheaper-model suggestions for the main loop |
| [`statusline`](#statusline) | One short line for Claude Code's status line |
| `hook <event>`, `mcp` | Internal: started by Claude Code, never by hand |

## Common behavior

- **Claude Code folder.** Commands that read or change it take `--claude-dir <PATH>`. The folder is the first that applies of: the flag, `[claude] dir` in `tokred.toml`, `$CLAUDE_CONFIG_DIR`, `~/.claude` (`%USERPROFILE%\.claude` on Windows). It is valid when it exists and holds `settings.json` or `projects/`. An explicit `--claude-dir` that is not valid is an error. [`initialize`](#initialize) and the [`proxy`](#proxy-enable-disable) commands, which edit the `settings.json` that Claude Code itself reads, put `$CLAUDE_CONFIG_DIR` before `[claude] dir`.
- **`--since`.** A span of hours, days or weeks: `12h`, `7d`, `2w`, `30d`.
- **`--json`.** Where a command lists it, the output is machine-readable. Not every command has it.
- **Exit codes.** `0` success, `1` error (the message goes to stderr), `2` for a usage error and for a `configure` the user declined. `tokred claude` exits with `claude`'s own code. `hook` always exits `0`; `initialize` fails only on invalid arguments.
- **Colors, environment.** Colors and the environment variables that change behavior are in [configuration](configuration.md).

## configure

```sh
tokred configure [--claude-dir <PATH>] [--yes] [--force]
```

Integrates tokred with Claude Code: the [hooks and agents](hooks-and-agents.md), plus the data folder, the database and a default `tokred.toml`. It does not set up the proxy; that is [`proxy enable`](#proxy-enable-disable).

1. Finds the Claude Code folder (see above). If none is found, a terminal is asked for the path; without a terminal the command fails and suggests `--claude-dir`.
2. Prints the plan: hooks to add (`+`), update (`~`) or keep (`=`), agents to create, update or leave alone, and the backup it will make.
3. Asks for confirmation. Answering no changes nothing and exits with code `2`, which the install script reads as "stop". Without a terminal the command refuses to run unless you pass `--yes`.
4. Applies the plan: creates the data folder, database and `tokred.toml`, merges `settings.json` (backup in `settings.json.tokred.bak`; only the last one is kept) and installs the agents.

Running it again is safe. If `settings.json` is not valid JSON the command stops and writes nothing. It never changes a `[proxy]` value you chose.

| Flag | Effect |
|---|---|
| `--claude-dir <PATH>` | Claude Code folder |
| `-y`, `--yes` | Apply without asking |
| `--force` | Overwrite agent files even if you edited them (otherwise your version is kept and the new one goes to a `.new` file) |

## unconfigure

```sh
tokred unconfigure [--claude-dir <PATH>] [--purge]
```

Removes tokred's hooks from `settings.json` and the agent files tokred installed. It touches only hooks and agents: not the proxy, not `ANTHROPIC_BASE_URL`, not your data. Edited agents are kept and reported, files without the `tokred-` prefix are never touched, and an upgrade does not put the hooks back: only `tokred configure` does. Details in [hooks and agents](hooks-and-agents.md).

| Flag | Effect |
|---|---|
| `--claude-dir <PATH>` | Claude Code folder |
| `--purge` | Also delete tokred's data folder (database, config, logs, reports); refused when the folder holds neither `tokred.db` nor `tokred.toml` |

## update

```sh
tokred update [--version <VERSION>] [--claude-dir <PATH>]
```

Checks the latest stable release (a pre-release is never offered) and acts according to where the running binary lives:

| The binary is | `tokred update` |
|---|---|
| In the install folder (`$TOKRED_HOME/bin`, where the install script puts it) | Downloads the archive for this platform, verifies its SHA-256, replaces the binary and re-applies the configuration with the new binary |
| Anywhere else (built from source, copied by hand) | Says whether a newer version exists and, if so, to reinstall with the install script |

`--version <VERSION>` (a leading `v` is fine) installs exactly that version instead of the latest stable one: use it to try a release candidate or to go back. It downloads that release directly and verifies the checksum like any update. A version that is already installed is not downloaded again. Needs the network and refuses to run when `TOKRED_OFFLINE` is set. The configuration is re-applied with the new binary and keeps your choices. See [installation](installation.md).

## uninstall

```sh
tokred uninstall [--claude-dir <PATH>]
```

Removes tokred from the machine completely and asks nothing. In order:

1. stops the proxy and removes its user service (if installed);
2. runs `compression off` (if the MCP server is registered);
3. removes tokred's `ANTHROPIC_BASE_URL` from Claude Code's `settings.json` (a gateway of your own stays);
4. removes the hooks and agents, like `unconfigure` (skipped when there is no Claude Code folder);
5. deletes tokred's data folder;
6. runs the uninstall script embedded in the binary: it deletes the binary and its folder (when nothing else is in it), the `PATH` entry and the `claude` wrapper in your shell profiles. A binary built from source is left where it is. On Windows this step runs right after the command exits, because a running program cannot delete itself.

Every step is best effort: a failure is printed and the next step still runs. If a step failed, the binary is kept so you can fix the problem and run the command again. See [installation](installation.md).

## doctor

```sh
tokred doctor [--claude-dir <PATH>] [--json] [--bench]
```

Prints the version, binary path, Claude Code and data folders, database schema, config status and whether other hook tools are present. Then a table of hooks (`ok`, `missing`, `outdated`, or `ok, but binary path is gone`), a table of agents (`current`, `outdated`, `edited by you`, `missing`, with `.new file pending` when one waits) and the list of problems and notes. A proxy section shows whether the [proxy](proxy.md) is running (or `disabled`), its routing and compression modes, the `ANTHROPIC_BASE_URL` in Claude Code's `settings.json` and in this terminal, and warns when `settings.json` points at a proxy that is not running. The HTTP proxy (host and port only) and extra CA files are shown when configured. For routing and compression still in `observe`, one line each says whether turning it on is worth it, from the last 7 days of measurements (an estimate; `--json` carries it as `proxy.savings`).

| Flag | Effect |
|---|---|
| `--claude-dir <PATH>` | Claude Code folder |
| `--json` | Machine-readable output |
| `--bench` | Also run every hook many times with realistic input and measure its real latency, from process start to exit. The result is saved and used by [`stats latency`](#stats-latency). `TOKRED_BENCH_RUNS` sets the runs per payload (default 50) |

## stats

```sh
tokred stats [--since <SPAN>] [--by <GROUP>] [--json] [--claude-dir <PATH>]
```

The cost report in the terminal: totals, what the same tokens would cost on opus, how far below an all-opus run your models put the cost (your choice, not tokred's), and what tokred saved with its applied routing and its guards (an estimate, labelled as such), the prompt-cache hit rate, the monthly projection and a table with a share column and an impact bar. How each number is computed is in [metrics](metrics.md). When the [proxy](proxy.md) recorded something, a **Proxy** section follows.

| Flag | Effect |
|---|---|
| `--since <SPAN>` | How far back to look (default `7d`) |
| `--by <GROUP>` | Group the rows: `model`, `day`, `project`, `origin` (main, subagent, auxiliary, compaction) or `agent-type` (agent type of a subagent request). `origin` and `agent-type` are exact only with the proxy |
| `--json` | Machine-readable output (adds a `proxy` key when the proxy recorded something) |

## stats latency

```sh
tokred stats latency [--since <SPAN>] [--by <GROUP>] [--json] [--claude-dir <PATH>]
```

How much time the hooks added to Claude Code: runs, p50/p95/p99/max per hook, total added time, average per prompt, outcomes and an alert when a hook's p95 passes 50 ms.

| Flag | Effect |
|---|---|
| `--since <SPAN>` | How far back to look (default `7d`) |
| `--by <GROUP>` | `hook`, `day` or `session` |
| `--json` | Machine-readable output |

## metrics

```sh
tokred metrics [--since <SPAN>] [--no-open] [--claude-dir <PATH>]
```

Prints the same report as `stats` and writes a self-contained HTML report to `<data>/reports/metrics.html` (always the same file, overwritten), then opens it in the browser. The report works offline. `--since` defaults to `30d`; `--no-open` only writes the file. See [metrics](metrics.md).

## audit

```sh
tokred audit [--project <PATH>] [--json] [--claude-dir <PATH>]
```

Estimates the weight of the context Claude Code loads before you type (`CLAUDE.md` files with their `@` imports, rules, skills, agents) and lists MCP servers that were not used in the last 30 days. `--project <PATH>` picks the project to audit (default: the current folder). Token counts are an estimate: characters divided by 4. The exact size of MCP tool schemas cannot be computed locally, so only usage is shown.

## pricing

```sh
tokred pricing sync
```

```sh
tokred pricing show [--json]
```

`sync` refreshes the price table from the public LiteLLM price list; if it cannot be reached, the embedded snapshot stays in use. `show` prints the table (USD per million tokens). Prices are also synced automatically, at most once a day, as described in [configuration](configuration.md). How prices are used is in [metrics](metrics.md).

## cache

```sh
tokred cache stats [--json]
```

```sh
tokred cache prune
```

`stats` shows entries and hits per tier (L1, L2, L3); `prune` removes expired entries. How the tiers work is in [architecture](architecture.md).

## proxy start, stop, restart, status

```sh
tokred proxy start [--foreground] [--bind <HOST:PORT>]
```

```sh
tokred proxy stop
```

```sh
tokred proxy restart
```

```sh
tokred proxy status [--json]
```

The optional [local proxy](proxy.md). `start` runs it in the background and returns as soon as it answers (it waits up to 5 seconds), so `out=$(tokred proxy start)` does not hang. `stop` asks it to shut down and waits until it has. `restart` does both, so that a setting the proxy reads when it starts applies (a proxy that was not running is just started); it refuses while the proxy features are off. `status` shows whether it is running, whether the user service is installed, whether this terminal's `ANTHROPIC_BASE_URL` points to it, and the routing and compression modes. While `[proxy] enabled = false`, `start` refuses; `stop` and `status` still work.

| Flag | Effect |
|---|---|
| `--foreground` | Run in this terminal instead of the background (what the user service uses) |
| `--bind <HOST:PORT>` | Address to listen on (default `[proxy] bind`, `127.0.0.1:1029`). Only loopback addresses are accepted |
| `--json` | `status` only: machine-readable output, including the HTTP proxy and CA files the proxy uses |

## proxy enable, disable

```sh
tokred proxy enable [--port <PORT>] [--record-content <BOOL>] [--yes]
```

```sh
tokred proxy disable
```

The one place that turns the proxy features on or off and applies it at once. They touch the proxy, `[proxy]` in `tokred.toml` and Claude Code's `env.ANTHROPIC_BASE_URL`: nothing else.

`enable` chooses two things, saves them with `[proxy] enabled = true` (in place: your other keys stay), starts the proxy and, once it answers, sets `env.ANTHROPIC_BASE_URL` in Claude Code's `settings.json` to its URL.

| Flag | Effect |
|---|---|
| `--port <PORT>` | Port to listen on, 1024 to 65535 (default: the saved one, else `1029`). Also `TOKRED_PORT`; the flag wins. Sets `[proxy] bind`, keeping its host |
| `--record-content <BOOL>` | Let the [dashboard](dashboard.md) show what was sent and returned: `true`, `false`, `yes`, `no`, `1` or `0`. Also `TOKRED_RECORD_CONTENT`; the flag wins. Sets `[proxy] log_bodies`. The text stays in tokred's own database on this machine; headers and credentials are never recorded |
| `-y`, `--yes` | Ask nothing: use the values given, the saved ones or the defaults |

In a terminal, a value that no flag or variable gave is asked: the port (a busy port is reported and asked again; the proxy of this installation counts as free) and whether to record conversation content (default: the saved answer, else yes). Without a terminal, or with `--yes`, nothing is asked: the saved values stay, and when none is saved the port is `1029` and content is **not** recorded. A proxy that is already running is restarted when the port or the recording answer changed, and the command says so.

`disable` sets `[proxy] enabled = false` and removes tokred's URL from `settings.json`. A running proxy keeps running (`tokred proxy stop` stops it). While disabled, `proxy start` and `proxy service install` refuse, `tokred claude` runs `claude` directly and `doctor` shows `disabled`. A different gateway in `ANTHROPIC_BASE_URL` is never overwritten or removed: `enable` then saves the setting and says so.

## proxy service

```sh
tokred proxy service install
```

```sh
tokred proxy service uninstall
```

Installs the proxy as a user service that starts at login (a systemd user unit on Linux, a launchd agent on macOS, a logon task on Windows), or removes it. Nothing is installed unless you run this. `install` refuses while `[proxy] enabled = false`.

## initialize

```sh
tokred initialize [--quiet | --verbose] [--json]
```

Makes Claude Code's `settings.json` agree with `[proxy] enabled`. The `claude` shell wrapper runs it before every launch, so it is fast, prints nothing unless there is a problem and exits `0` even when it cannot do its job. Enabled: starts the proxy when it is not running and sets `env.ANTHROPIC_BASE_URL` once the proxy answers (or removes it again, with a warning, if the proxy cannot start). Disabled: removes tokred's URL. A gateway of your own, in the file or exported in the shell, is never touched.

| Flag | Effect |
|---|---|
| `--quiet` | The default: print only warnings |
| `-v`, `--verbose` | Also say what changed |
| `--json` | Report on stdout: `state`, `started`, `actions`, `warnings` |

## claude

```sh
tokred claude [ARGS...]
```

Starts the proxy if needed and runs Claude Code through it, for this run only: nothing is written to Claude Code's settings. Every argument goes to `claude` unchanged, except `-h`, `--help`, `-V` and `--version`, which tokred answers itself (put `--` before them to pass them on). If the proxy cannot start, `ANTHROPIC_BASE_URL` already points to another gateway, or `[proxy] enabled = false`, Claude Code runs as before and tokred says why on stderr.

## dashboard

```sh
tokred dashboard [--no-open]
```

Opens the [live dashboard](dashboard.md) of the running proxy in the browser. It starts nothing: if the proxy is not running it says so. The address carries a one-time token that expires after about a minute; run the command again for a new one. `--no-open` prints the address instead of opening it.

## routing

```sh
tokred routing status [--since <SPAN>] [--json]
```

```sh
tokred routing on
```

```sh
tokred routing off
```

```sh
tokred routing explain <ID>
```

`status` shows the mode, what routing did (or would have done, in `observe`) over the period (`--since`, default `7d`: requests changed, estimated saving and a monthly projection, so you can decide whether to turn it on) and the recent decisions. `on` sets `[routing] mode = "enforce"` and `off` sets `"observe"`: both edit `tokred.toml` and restart a running proxy to apply it. `explain <ID>` says why one decision was taken (ids are in `status`). The main loop's model is never changed. See [routing](routing.md).

## compression

```sh
tokred compression on [--claude-dir <PATH>]
```

```sh
tokred compression off [--claude-dir <PATH>]
```

```sh
tokred compression status
```

`on` registers the `tokred_retrieve` MCP server in Claude Code's user config (`.claude.json` in the Claude Code folder if it exists there, else `~/.claude.json`; the previous file is saved next to it as `.tokred.bak`) and sets `[compression] mode = "enforce"`: large tool results are replaced by a preview and the model reads the original back with `tokred_retrieve`. `off` unregisters the server and sets `"observe"` (measure only). Restart the proxy and Claude Code to apply `on`. `status` shows the mode, whether the server is registered, whether the circuit breaker tripped and, while it is still in `observe`, what compression would have kept out of the requests in the last 7 days (the share of the input tokens, so you can tell whether it is worth turning on). Nothing is edited unless you run `on` or `off`.

## model-guard

```sh
tokred model-guard on
```

```sh
tokred model-guard warn
```

```sh
tokred model-guard off
```

```sh
tokred model-guard limit <TOKENS>
```

```sh
tokred model-guard status
```

Sets the mode and the limit of the `model-guard` [hook](hooks-and-agents.md#what-each-hook-decides) in `tokred.toml`. `on` asks you to confirm a model switch that would throw away a warm prompt cache, `warn` (the default) only says what it costs and lets the switch go ahead, and `off` does nothing. `limit` sets `[model_guard] min_context_tokens`, the context size from which a switch is reported (100,000 by default). `status` prints the mode and the limit. The hook reads the file every time it runs, so a change applies at once: nothing is restarted. It needs Claude Code 2.1.251 or later.

## suggest

```sh
tokred suggest [--since <SPAN>] [--json]
```

How many prompts the proxy tracked, how many were simple, and the latest suggestions of a cheaper model for the main loop, with the evidence. It only advises; switching is always your call (`/model`). Needs the proxy. `--since` defaults to `7d`.

## statusline

```sh
tokred statusline
```

Reads Claude Code's status-line JSON on stdin and prints one short line: the pending suggestion and a few session figures. Set it as your status-line command if you want it.

## Internal commands

`tokred hook <event>` is called by Claude Code, never by hand (see [hooks and agents](hooks-and-agents.md); for testing, see [development](development.md)). It reads JSON on stdin, writes JSON on stdout and always exits `0`, even for an unknown event. `tokred mcp` is the `tokred_retrieve` MCP server that Claude Code starts when compression is on.
