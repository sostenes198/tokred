**English** | [Português](configuration.pt-BR.md)

# Configuration

## Locations

| Item | Source |
|---|---|
| Claude Code folder | `--claude-dir`, then `[claude] dir` in `tokred.toml`, then `$CLAUDE_CONFIG_DIR`, then `~/.claude` (Windows: `%USERPROFILE%\.claude`) |
| tokred data folder | `$TOKRED_DATA_DIR` if set; otherwise Linux `~/.local/share/tokred`, macOS `~/Library/Application Support/tokred`, Windows `%APPDATA%\tokred` |
| Database | `<data>/tokred.db` |
| Config | `<data>/tokred.toml` |
| Logs | `<data>/logs/` (daily rotation, 7 days kept) |
| Reports | `<data>/reports/metrics.html` |

## Environment variables

| Variable | Effect |
|---|---|
| `TOKRED_DATA_DIR` | Overrides the data folder (also used by the tests) |
| `CLAUDE_CONFIG_DIR` | Claude Code folder, as Claude Code itself reads it |
| `TOKRED_LOG` | Log level written to the log file: `error`, `warn`, `info` (default), `debug`, `trace` |
| `TOKRED_NO_UPDATE_CHECK` | `1` disables the daily version check |
| `TOKRED_OFFLINE` | `1` disables every network call (version check, price sync, `update`, `pricing sync`) |
| `TOKRED_PRICES_URL` | Alternative URL of the LiteLLM price list (mirrors, tests) |
| `TOKRED_RELEASES_URL` | Alternative URL of the "latest release" API (mirrors, tests) |
| `TOKRED_BENCH_RUNS` | Runs per payload of `tokred doctor --bench` (default 50) |
| `TOKRED_HOOK_GUARD_MS` | Overrides the 2 s time limit of a hook (used by the tests) |
| `TOKRED_PROXY_OVERHEAD_REPORT` | Used by the proxy overhead test only: file to write the measured percentiles to |
| `NO_COLOR` / `CLICOLOR_FORCE` | Turn terminal colors off / force them on |

## `tokred.toml`

Created by `configure` with every key below. On update, new keys are added with their default and **existing values are preserved**; the file is only rewritten when a key was missing (so your comments survive otherwise).

```toml
[claude]
dir = "/Users/you/.claude"   # saved by `tokred configure`

[modes]
route = "warn"          # off | warn | enforce
enforce = "warn"
guard_read = "enforce"
guard_search = "warn"
session_check = "warn"

[guard_read]
max_read_bytes = 204800
deny_globs = ["**/package-lock.json", "**/pnpm-lock.yaml", "**/yarn.lock",
              "**/Cargo.lock", "**/poetry.lock", "**/*.min.js", "**/*.map",
              "**/dist/**", "**/build/**", "**/node_modules/**", "**/target/**", "**/.git/**"]

[guard_search]
large_repo_files = 5000

[session_check]
context_warn_tokens = 120000

[update]
mode = "notify"         # off | notify | auto

[pricing]
auto_sync = true

[cache]
l1_ttl = "4h"
l2_ttl = "24h"
l2_hits = 3
l3_hits = 10

[proxy]
enabled = false                        # reserved: not read yet
bind = "127.0.0.1:8787"                # address the proxy listens on
upstream = "https://api.anthropic.com" # the only host requests are forwarded to
persist_base_url = false               # reserved: not read yet
retention_days = 90                    # reserved: not read yet
log_bodies = false                     # reserved: not read yet
# ca_bundle = "/etc/ssl/corp-ca.pem"   # optional (not written by default): extra CA certificates (PEM) to trust

[routing]
mode = "observe"                       # off | observe | enforce
min_confidence = 0.8                   # minimum classification confidence (0..1) to act
min_saving_usd = 0.005                 # minimum estimated saving of one decision
max_rewrite_failure_rate = 0.05        # rejected-rewrite rate that opens the circuit breaker
cooldown = "30m"                       # how long the circuit breaker stays open
allowed_models = []                    # empty: any cheaper model of the catalog
compaction = "keep"                    # keep | cheap (opt-in)

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

[shadow]
compression = true                     # measure what compression would save
response_cache = true                  # count exact repeated requests
```

The `[modes]` values take effect when the corresponding hook gets its real behaviour (phases F3 to F5).

`ca_bundle` in `[proxy]` is optional and absent by default: the path of a PEM file with extra certificate authorities to trust for the upstream, in addition to the built-in roots, the operating system's store and the files named by `NODE_EXTRA_CA_CERTS` and `SSL_CERT_FILE`. The proxy also honours `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` and `NO_PROXY` (and the lower-case names) from its environment; see [behind a corporate proxy](proxy.md#behind-a-corporate-proxy).

The proxy sections only matter when the [proxy](proxy.md) runs. `tokred routing on|off` and `tokred compression on|off` set `mode` for you (restart the proxy to apply). `routing` and `compression` default to `observe`: they record what they would do and change nothing. `[routing]` never changes the main loop's model, whatever it says; see [routing](routing.md). `channel` in `[suggest]`: `stats` (default) makes them available in `tokred suggest`, `statusline` adds the status line, `message` also allows a one-line message at a safe moment (see [hooks](hooks.md)). `[shadow]` measurements never alter traffic.
