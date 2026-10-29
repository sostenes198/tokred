**English** | [Português](CHANGELOG.pt-BR.md)

# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed

- `tokred stats`, `tokred metrics` and the HTML report read the proxy's exact per-request data as their primary source and the transcripts for what the proxy did not see, merged so no request is counted twice ([metrics](docs/metrics.md)). Without the proxy nothing changes.
- `unconfigure` leaves settings it does not understand untouched instead of failing.

### Fixed

- Cache writes with the 1-hour lifetime are priced at their own rate (2x input) instead of the 5-minute rate (1.25x); `stats` previously under-reported them. Existing usage is backfilled automatically on the next `stats` run.
- The "latest model of a family" (used for the all-on-opus comparison and for unknown models) now ranks by version before date, so current undated ids such as `claude-opus-5-5` are no longer outranked by dated 4.x ids.
- Fable and Mythos are recognized as model families (price fallback, report colors).
- Transcript ingestion on Windows read stale file sizes while Claude Code still had the transcript open.

- Hook commands are recognized for any executable whose name starts with `tokred` (renamed or versioned binaries no longer cause duplicate entries).

### Added

- Optional local proxy between Claude Code and the Anthropic API, off until used ([proxy](docs/proxy.md)): exact measurement of every request (model, tokens including 1-hour cache writes, latency, cache-miss cause, proxy overhead); routing of subagent and auxiliary requests to a cheaper model (`observe` by default; the main loop's model is never changed); advice about the main loop (`tokred suggest`, `tokred statusline`, and an optional one-line message from the `route` hook); reversible compression of large tool results with the `tokred_retrieve` MCP server (`observe` by default). It is fail-open: if anything fails, the original request is forwarded unchanged.
- `tokred claude [args...]`: starts the proxy if needed and runs Claude Code through it for this run only, passing every argument through.
- `tokred proxy start|stop|status` and `tokred proxy service install|uninstall`; `tokred routing status|on|off|explain`; `tokred compression on|off|status` (`on` registers the MCP server in `~/.claude.json` with a `.tokred.bak` backup).
- `tokred doctor` shows the proxy (running, modes, where `ANTHROPIC_BASE_URL` points); `TOKRED_BENCH_RUNS` sets the runs of `doctor --bench`.
- With the proxy recording: `tokred stats --by origin` and `--by agent-type`, and a Proxy section in `stats`, `metrics` and the HTML report with the overhead against its 10 ms budget, routing decisions with observed and applied savings, the model asked for against the one that answered, prompt-cache misses by cause, and the latest pending suggestion (`stats --json` gets a `proxy` key).
- New `[proxy]`, `[routing]`, `[routing.tiers]`, `[suggest]`, `[compression]` and `[shadow]` sections in `tokred.toml` ([configuration](docs/configuration.md)).
- Phases F1 to F5: colored terminal dashboards (`tokred stats`, `stats latency`, `metrics`) with an offline HTML report; price table with an embedded snapshot and a daily sync; the daily version check and `tokred update` (checksum-verified self-update for script installs); `tokred audit`; `tokred cache`; the six hooks (`guard-read`, `guard-search`, `route`, `enforce`, `session-check`, `session-start`) with latency recording and `doctor --bench`; the `dist` release pipeline with Homebrew, Scoop, winget and crates.io publishing.
- `TOKRED_OFFLINE`, `TOKRED_PRICES_URL`, `TOKRED_RELEASES_URL`, `NO_COLOR` and `CLICOLOR_FORCE`.
- Phase F0: workspace bootstrap (ten crates, CI, cargo-deny, bilingual docs).
- `tokred configure`: detects the Claude Code folder, shows a plan, merges tokred hooks into `settings.json` without touching anything else, installs six subagents, creates the SQLite database and `tokred.toml`.
- `tokred unconfigure [--purge]`: removes only what tokred added.
- `tokred doctor [--json]`: reports hooks, agents, rtk detection and problems.
- `tokred hook <event>`: fail-open stub for the six hooks (real behaviour arrives in phases F3 to F5).
