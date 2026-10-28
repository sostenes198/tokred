**English** | [Português](cache.pt-BR.md)

# Store and cache

tokred keeps its state in one SQLite file, `<data>/tokred.db`, opened with `journal_mode=WAL` and `busy_timeout=2000`. The schema is versioned with `PRAGMA user_version`; each migration runs in its own transaction, and a database created by a newer tokred is refused rather than downgraded.

## Tables

| Table | Holds | Written by |
|---|---|---|
| `sessions` | Session id, working directory, first/last seen, last reset (`clear`/`compact`) | `session-start` |
| `classifications` | Actionable prompt class per session (never `other`) | `route` |
| `file_reads` | `(scope, path, hash, offset, limit)` reads and estimated bytes; an absent `offset`/`limit` is stored as `-1`; the scope is the session id, plus `/<agent_id>` inside a subagent | `guard-read` |
| `hook_events` | Every hook run: decision, reason, bytes saved, duration in µs, outcome | every hook |
| `cache_entries` | Tiered cache (below) | `guard-search`, `session-check` |
| `installed_files` | Files tokred installed, with blake3 hash and version | `configure` |
| `pricing` | USD per million tokens, per model | pricing sync |
| `usage` | Per-message token usage from transcripts | ingestion |
| `ingest_state` | Incremental ingestion offsets | ingestion |
| `meta` | Small key/value state (version check, price sync, bench) | CLI |

### `hook_events`

One row per hook run (nothing is recorded for `off` mode, unknown hook names, or when there is no database). `decision` is `allow`, `warn`, `deny`, `none`, or, for `route`, the task class. `reason` is one of `reread`, `generated_file`, `large_file`, `broad_search`, `agent_matches`, `agent_mismatch`, `suggested`, `unchanged`, `context_large`, `already_warned`, `below_limit`, the `source` of a `session-start`, or the `outcome` when the run failed. `outcome` is `ok`, `error`, `panic` or `timeout_guard`. `duration_us` is measured from the first line of `main` until just before the row is written, which is the last step of the run.

## Tiered cache (`cache_entries`)

| Tier | Enters when | TTL |
|---|---|---|
| L1 | First write | `l1_ttl` (default 4 h), counted from creation |
| L2 | `hits >= l2_hits` (default 3) | `l2_ttl` (default 24 h), renewed on every hit |
| L3 | `hits >= l3_hits` (default 10) | Persistent (`expires_at = NULL`) |

- Keys always include the hash of the source content, so a changed source invalidates its entries automatically: `<kind>:<hash>:<subject>`. For file counts the "content" is the repository root's modification time (and the threshold), which is the cheapest signal that the repository changed.
- Promotion happens on read (`cache_get`): it increments `hits`, re-evaluates the tier and renews the expiry of L2 entries. `cache_peek` reads without counting a hit. An expired entry is a miss.
- An unreadable `l1_ttl` / `l2_ttl` in `tokred.toml` (anything but a number plus `s`, `m`, `h` or `d`) falls back to its default instead of breaking a hook.
- The policy is made of pure functions in `tokred-core` (`next_tier`, `expires_at`, `is_expired`, `record_hit`); the store only loads and saves rows.
- `tokred cache stats` shows entries, expired entries, hits and bytes per tier; `tokred cache prune` removes expired entries. Pruning also runs opportunistically from `stats` and `update`, **never inside a hook**.
- What the cache holds today: the per-root file counts of `guard-search` (kind `repo_files`) and the "already warned" markers of `session-check` (kind `session_warn`). Classifications live in their own table, because a lookup by session is cheaper than hashing the prompt, and the audit result arrives with the audit phase. Artifact summaries come in phase F6.
