**English** | [Português](CHANGELOG.pt-BR.md)

# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed

- `unconfigure` leaves settings it does not understand untouched instead of failing.

### Fixed

- Cache writes with the 1-hour lifetime are priced at their own rate (2x input) instead of the 5-minute rate (1.25x); `stats` previously under-reported them. Existing usage is backfilled automatically on the next `stats` run.
- The "latest model of a family" (used for the all-on-opus comparison and for unknown models) now ranks by version before date, so current undated ids such as `claude-opus-5-5` are no longer outranked by dated 4.x ids.
- Fable and Mythos are recognized as model families (price fallback, report colors).
- Transcript ingestion on Windows read stale file sizes while Claude Code still had the transcript open.

- Hook commands are recognized for any executable whose name starts with `tokred` (renamed or versioned binaries no longer cause duplicate entries).

### Added

- Phases F1 to F5: colored terminal dashboards (`tokred stats`, `stats latency`, `metrics`) with an offline HTML report; price table with an embedded snapshot and a daily sync; the daily version check and `tokred update` (checksum-verified self-update for script installs); `tokred audit`; `tokred cache`; the six hooks (`guard-read`, `guard-search`, `route`, `enforce`, `session-check`, `session-start`) with latency recording and `doctor --bench`; the `dist` release pipeline with Homebrew, Scoop, winget and crates.io publishing.
- `TOKRED_OFFLINE`, `TOKRED_PRICES_URL`, `TOKRED_RELEASES_URL`, `NO_COLOR` and `CLICOLOR_FORCE`.
- Phase F0: workspace bootstrap (ten crates, CI, cargo-deny, bilingual docs).
- `tokred configure`: detects the Claude Code folder, shows a plan, merges tokred hooks into `settings.json` without touching anything else, installs six subagents, creates the SQLite database and `tokred.toml`.
- `tokred unconfigure [--purge]`: removes only what tokred added.
- `tokred doctor [--json]`: reports hooks, agents, rtk detection and problems.
- `tokred hook <event>`: fail-open stub for the six hooks (real behaviour arrives in phases F3 to F5).
