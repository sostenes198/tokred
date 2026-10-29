**English** | [Português](configuration.pt-BR.md)

# Configuration

Where tokred keeps its files, which environment variables it reads, and every key of `tokred.toml` with its default.

## Locations

| Item | Location |
|---|---|
| Claude Code folder | `--claude-dir`, then `[claude] dir` in `tokred.toml`, then `$CLAUDE_CONFIG_DIR`, then `~/.claude` (Windows: `%USERPROFILE%\.claude`) |
| tokred data folder | `$TOKRED_DATA_DIR` if set; otherwise Linux `~/.local/share/tokred`, macOS `~/Library/Application Support/tokred`, Windows `%APPDATA%\tokred` |
| Database | `<data>/tokred.db` |
| Config | `<data>/tokred.toml` |
| Logs | `<data>/logs/` (one file per day, 7 days kept) |
| HTML report | `<data>/reports/metrics.html` |
| Proxy runtime files | `<data>/proxy/` (pid file, admin secret) |

## Environment variables

| Variable | Effect |
|---|---|
| `TOKRED_DATA_DIR` | Overrides the data folder |
| `CLAUDE_CONFIG_DIR` | Claude Code folder, as Claude Code itself reads it |
| `TOKRED_LOG` | Level written to the log file: `error`, `warn`, `info` (default), `debug`, `trace` |
| `TOKRED_OFFLINE` | `1` disables every network call: version check, price sync, `update`, `pricing sync` |
| `TOKRED_NO_UPDATE_CHECK` | `1` disables the daily version check only |
| `TOKRED_PORT` | Proxy port for `tokred proxy enable` and the install script (1024 to 65535); the `--port` flag wins |
| `TOKRED_RECORD_CONTENT` | Answers `tokred proxy enable`'s question about recording conversation content (`true`, `false`, `yes`, `no`, `1`, `0`) without asking; the `--record-content` flag wins |
| `TOKRED_BENCH_RUNS` | Runs per payload of `tokred doctor --bench` (default 50) |
| `TOKRED_PRICES_URL` | Alternative URL of the LiteLLM price list (mirrors) |
| `TOKRED_RELEASES_URL` | Alternative URL of the "latest release" API (mirrors) |
| `TOKRED_HOOK_GUARD_MS` | Overrides the 2 s time limit of a hook, in milliseconds (meant for tests) |
| `TOKRED_HOME` | Where the install script put the binary (`$TOKRED_HOME/bin`; default `~/.tokred`, Windows `%LOCALAPPDATA%\tokred`). `tokred update` only replaces a binary that lives there |
| `NO_COLOR` | Any non-empty value turns terminal colors off |
| `CLICOLOR_FORCE` | Any non-empty value other than `0` forces colors on, even when stdout is not a terminal |

The install and uninstall scripts also read `TOKRED_VERSION`, `TOKRED_YES`, `TOKRED_NO_MODIFY_PATH`, `TOKRED_NO_ALIAS`, `TOKRED_NO_PROXY`, `TOKRED_NO_CONFIGURE` and `TOKRED_RELEASE_BASE_URL`; they are described in [installation](installation.md). The proxy reads the usual network variables (`HTTPS_PROXY`, `NO_PROXY`, `NODE_EXTRA_CA_CERTS`, ...), described in [proxy](proxy.md).

## tokred.toml

`tokred configure` creates the file with every key at its default. The file it writes has no comments and its keys are sorted alphabetically; the order and the comments below are only for reading. Edit it with any editor; a restart of the proxy applies `[proxy]`, `[routing]`, `[suggest]`, `[compression]` and `[shadow]` changes, and hooks read the file on every run.

**How updates merge it.** On `configure` and after an upgrade, keys that are missing are added with their default and every value you set is kept. The file is rewritten only when a key was missing, so your comments survive otherwise. A key that tokred no longer knows is ignored. A `tokred.toml` that cannot be parsed makes the hooks use the defaults.

```toml
[claude]
dir = "/Users/you/.claude"   # saved by `tokred configure`

[modes]                      # off | warn | enforce, one per hook
route = "warn"
enforce = "warn"
guard_read = "enforce"
guard_search = "warn"
session_check = "warn"

[guard_read]
max_read_bytes = 204800      # larger files need offset/limit
deny_globs = ["**/package-lock.json", "**/pnpm-lock.yaml", "**/yarn.lock",
              "**/Cargo.lock", "**/poetry.lock", "**/*.min.js", "**/*.map",
              "**/dist/**", "**/build/**", "**/node_modules/**", "**/target/**", "**/.git/**"]

[guard_search]
large_repo_files = 5000      # a repository with more files counts as large

[session_check]
context_warn_tokens = 120000 # warn the user when the context passes this

[update]
mode = "notify"              # off | notify | auto (auto currently behaves like notify)

[pricing]
auto_sync = true             # refresh the price table at most once a day

[cache]
l1_ttl = "4h"                # tiered cache, see architecture.md
l2_ttl = "24h"
l2_hits = 3                  # hits needed to reach L2
l3_hits = 10                 # hits needed to reach L3

[proxy]
enabled = false                        # the switch of the proxy features
bind = "127.0.0.1:1029"                # address the proxy listens on
upstream = "https://api.anthropic.com" # the only host requests are forwarded to
retention_days = 90                    # days of proxy records and captured dashboard turns kept; 0 keeps everything
log_bodies = false                     # record conversation content for the dashboard (off: metadata only)
# ca_bundle = "/etc/ssl/corp-ca.pem"   # optional, absent by default: extra CA certificates (PEM) to trust

[routing]
mode = "observe"                       # off | observe | enforce
min_confidence = 0.8                   # minimum classification confidence (0..1) to act
min_saving_usd = 0.005                 # minimum estimated saving of one decision
max_rewrite_failure_rate = 0.05        # rejected-rewrite rate that opens the circuit breaker
cooldown = "30m"                       # how long the circuit breaker stays open
allowed_models = []                    # empty: any cheaper model of the catalog
compaction = "keep"                    # keep | cheap

[routing.tiers]                        # task class -> cheap | mid | keep
search = "cheap"
summary = "cheap"
implementation = "mid"
planning = "keep"
architecture = "keep"
review = "keep"
auxiliary = "cheap"

[suggest]
channel = "stats"                      # off | stats | statusline | message
min_simple_prompts = 3                 # simple prompts in a row before a suggestion
cooldown = "6h"                        # minimum time between two suggestions in a session

[compression]
mode = "observe"                       # off | observe | enforce
min_tokens = 1500                      # only tool results above this (bytes / 4) are replaced
preview_head_lines = 40
preview_tail_lines = 20
retention = "7d"                       # how long originals are kept for `tokred_retrieve`
max_retrievals = 3                     # retrievals of one id before its tool is excluded

[shadow]                               # measurements that never alter traffic
compression = true                     # measure what compression would save
response_cache = true                  # count exact repeated requests
```

### Notes by section

| Section | What to know |
|---|---|
| `[modes]` | What each mode does is in [hooks and agents](hooks-and-agents.md). `route` and `session_check` never deny, so `enforce` behaves like `warn` for them. |
| `[guard_read]`, `[guard_search]`, `[session_check]` | Thresholds of the hooks of the same name. `deny_globs` are matched against the path relative to the session's folder. |
| `[update]` | `off` never checks for new versions; `notify` prints a notice (at most one request a day). `auto` is accepted and behaves like `notify`: nothing updates itself, run `tokred update`. |
| `[pricing]` | With `auto_sync = false` prices change only with `tokred pricing sync`. |
| `[cache]` | Durations take `s`, `m`, `h` or `d` (`90s`, `30m`, `4h`, `7d`). |
| `[proxy]` | `tokred proxy enable` writes `enabled`, `bind` (it changes the port and keeps the host) and `log_bodies` for you; `configure` never changes them. While `enabled = false`, tokred removes its own `ANTHROPIC_BASE_URL` from Claude Code's settings, `tokred claude` runs `claude` directly, `proxy start` refuses and `doctor` shows `disabled`. `retention_days` applies to everything the proxy stores (requests, routing decisions, summaries, suggestions, measurements, kept tool results) and runs when the proxy starts and then every hour. With `log_bodies = false` only metadata is recorded, never message text. `ca_bundle` is added to the built-in roots, the system store and the files named by `NODE_EXTRA_CA_CERTS` and `SSL_CERT_FILE`. See [proxy](proxy.md). |
| `[routing]`, `[routing.tiers]` | Never change the main loop's model, whatever they say. `observe` records what routing would do and changes nothing. `tokred routing on` / `off` set `mode` for you. See [routing](routing.md). |
| `[suggest]` | `off` stops suggestions from being produced. Any other value makes them available in `tokred suggest` and, if you set `tokred statusline` as your status line, there too. `message` also lets a one-line message reach you at a safe moment (see [hooks and agents](hooks-and-agents.md)). Needs the proxy. |
| `[compression]` | `tokred compression on` / `off` set `mode` for you. With `observe` (the default) tokred only measures. |
| `[shadow]` | Measurements for the report; they never alter traffic. |
