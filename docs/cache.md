**English** | [Português](cache.pt-BR.md)

# Store and cache

tokred keeps its state in one SQLite file, `<data>/tokred.db`, opened with `journal_mode=WAL` and `busy_timeout=2000`. The schema is versioned with `PRAGMA user_version`; each migration runs in its own transaction, and a database created by a newer tokred is refused rather than downgraded.

## Tables (schema v1)

| Table | Holds | Used from |
|---|---|---|
| `sessions` | Session id, project, first/last seen, last reset (`clear`/`compact`) | F3 |
| `classifications` | Prompt class per session | F4 |
| `file_reads` | `(session, path, hash, offset, limit)` reads and bytes saved | F3 |
| `hook_events` | Every hook run: decision, reason, bytes saved, duration in µs, outcome | F3 |
| `cache_entries` | Tiered cache | F1 |
| `installed_files` | Files tokred installed, with blake3 hash and version | **F0** |
| `pricing` | USD per million tokens, per model | F1 |
| `usage` | Per-message token usage from transcripts | F1 |
| `ingest_state` | Incremental ingestion offsets | F1 |

All tables exist from the first migration, so later phases need no schema change just to start writing.

## Tiered cache (`cache_entries`)

| Tier | Enters when | TTL |
|---|---|---|
| L1 | First write | `l1_ttl` (default 4 h) |
| L2 | `hits >= l2_hits` (default 3) | `l2_ttl` (default 24 h), renewed on every hit |
| L3 | `hits >= l3_hits` (default 10) | Persistent (`expires_at = NULL`) |

- Keys always include the hash of the source content, so a changed file invalidates its entries automatically.
- Promotion happens on read: it increments `hits` and re-evaluates the tier.
- `tokred cache prune` removes expired entries; it also runs opportunistically from `stats` and `update`, never inside a hook.
- In the MVP the cache holds per-root file counts, classifications and the audit result. Artifact summaries come in phase F6.
