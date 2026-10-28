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
```

The `[modes]` values take effect when the corresponding hook gets its real behaviour (phases F3 to F5).
