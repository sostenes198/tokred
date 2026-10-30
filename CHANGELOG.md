**English** | [Português](CHANGELOG.pt-BR.md)

# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [1.5.0-rc.1] - 2026-10-30

### Changed

- `tokred stats` and the HTML report no longer call the distance to an "everything on opus" run a routing saving. That figure comes from the models you run, not from tokred, so it has its own line (**Model choice vs opus**), and the total became **Saved by tokred**: the saving of the routing decisions that were applied plus the guards, which is $0.00 while routing only observes. The efficiency meter is now **Below all-opus cost**. In `tokred stats --json`, `routing_saving_usd` holds what the applied decisions saved, the old value is `model_choice_saving_usd`, and `total_saving_usd` is always a number. `tokred stats latency` says "saved by tokred" in its trade-off line.
- The latency card of the report shows the average time one hook run adds, with the total over the period in its note. "Latency added, in total" read as a delay on every use.

### Fixed

- Claude Haiku 5.5 is priced by prompt length: a request whose prompt is over 100,000 tokens pays five times every price, as Anthropic publishes. The real cost, the proxy's model table, the dashboard and the Haiku estimate of a suggestion now use it. Before, every Haiku 5.5 prompt was priced at the cheap rate, so the saving suggested for a long session was overstated.

## [1.4.0] - 2026-10-30

### Added

- `tokred model-guard on | warn | off | limit <tokens> | status` sets the mode and the limit of the `model-guard` hook without editing `tokred.toml`. It applies at once.

## [1.3.0] - 2026-10-30

### Added

- `model-guard`, a seventh hook (on Claude Code's `PreModelSwitch` event): when you switch models (`/model`, the model picker, `/config` or fast mode) while the prompt cache is warm and the context has at least `[model_guard] min_context_tokens` tokens (100,000 by default), it says what the switch costs (`warn`, the default) or asks you to confirm it (`on`). It uses the figures Claude Code sends, never blocks by itself and needs Claude Code 2.1.251 or later.

### Changed

- `on` is accepted as another name for `enforce` in `[modes]`.

## [1.2.0] - 2026-10-29

### Added

- `tokred doctor` and `tokred compression status` say whether turning routing and compression on is worth it, from the last 7 days of measurements (an estimate): for routing, the requests it would have changed and the saving; for compression, the share of the input tokens it would have kept out. `doctor --json` carries it as `proxy.savings`.
- A savings hint: when the measurements show that turning routing or compression on is clearly worth it (routing: more than about $1 in 7 days; compression: more than 5% of the input tokens), tokred says so once a week in one line in your chat that only you see and that never reaches the model. Nothing is shown when the figures are small, for a feature that is already on, while `[suggest] channel = "off"`, or with `[suggest] savings_hint = false`.

## [1.1.0] - 2026-10-29

### Added

- `tokred proxy restart` stops the proxy and starts it again, so that a setting it reads at start applies (a proxy that was not running is just started).
- `tokred routing status` now answers whether routing is worth turning on: over a period (`--since`, default `7d`) it says how many requests routing changed or would have changed, the estimated saving and a monthly projection.
- A page that lists every switch (hooks and agents, the proxy, routing, compression, suggestions, updates, the `claude` wrapper), what each one changes and when it takes effect.

### Changed

- `tokred routing on` and `tokred routing off` restart a running proxy to apply the mode instead of asking you to.
- The dashboard sign-in lasts: the browser keeps the session for 30 days and it survives a proxy restart, so the link from `tokred dashboard` is needed once per browser. To sign every browser out, stop the proxy, delete `proxy/secret` in the data folder and start it again.

## [1.0.1] - 2026-10-29

### Fixed

- The cache-miss report no longer counts the switches between the conversation and Claude Code's side queries as misses. A session can interleave requests of several models and system prompts that the proxy sees as the main loop, each with a cache of its own, and the proxy compared each with the last one of another kind (most of the "compaction" and "model_changed" misses were that). A request is now compared with the previous one of its own series (same model and system prompt); the first request of a series is still a miss, with its cause.

### Changed

- The dashboard shows, on each request, its series and the cause of a prompt-cache miss; the metrics report explains each cause and the per-series comparison.

## [1.0.0] - 2026-10-29

### Fixed

- The release smoke test no longer stalls on macOS runners: its fake upstream skips the reverse DNS lookup that `http.server` does when it binds, which made it miss its start-up deadline.

## [1.0.0-rc.1] - 2026-10-29

### Changed

- The release pipeline deletes the release candidates of a version (their pre-releases and tags) once its final release passes the smoke test, and that test now proves `tokred update --version` by replacing the binary of a second installation with another published version and back.

## [0.1.0] - 2026-10-29

### Fixed

- `tokred uninstall` no longer refuses to delete a data folder that holds only the proxy's files (no database or `tokred.toml` yet).
- Clearer messages: `proxy enable` says conversation content "will not be recorded", the proxy's refusal to listen on a non-loopback address no longer suggests a `--bind` that cannot allow it, and `routing status --help` no longer promises a circuit breaker it does not show.
- `tokred uninstall` no longer runs its final cleanup from a fixed file in the shared temporary folder: the script is written to a new, uniquely named file that only its owner can read (another user could have planted a file at the fixed name, and two uninstalls at once collided). The Windows cleanup now removes the PATH entry and the profile wrapper before the binary, so the binary's folder disappearing means it is done.

## [0.1.0-rc.3] - 2026-10-29

### Fixed

- A proxy that was still shutting down when its replacement started erased the replacement's pid file, so `tokred proxy status` showed no pid and `tokred proxy enable` could not find the running proxy: turning conversation recording off after a `stop`/`enable` went unapplied. A proxy now removes the pid file only if it still names it, and `enable` also looks for the proxy on the configured port, accepting only one that answers with this installation's secret.

## [0.1.0-rc.2] - 2026-10-29

### Added

- `tokred update --version 0.2.0-rc.3` installs exactly that version (a release candidate, or an older one) instead of the latest stable release, which never includes pre-releases.

- The dashboard names each session the way Claude Code does (the title you gave it or the generated one, plus the folder and a short id), instead of showing only the session id.

### Fixed

- `tokred proxy enable --record-content false` now takes effect on a proxy that is already running: the proxy reads that setting only when it starts, so it kept recording conversation content until restarted. `enable` now asks the running proxy what it records (`content_recorded` in its status) and restarts it when that differs from what was asked, or when it is too old to say.

- The metrics report's summary cards (real cost, savings, tokens avoided, projection, latency) now follow the 7 days / 30 days / All buttons, with the period each one covers; before, only the daily charts changed. The toolbar says "Summary and daily charts"; the model, routing, guard, latency and audit sections still cover the whole period.

- `tokred update` says that no stable release has been published yet when only pre-releases exist (GitHub's "latest" answers 404 then), instead of failing with `http status: 404`.

## [0.1.0-rc.1] - 2026-10-29

### Added

- One-command install: `install/install.sh` (macOS, Linux) and `install/install.ps1` (Windows) download the release archive from GitHub, verify its checksum, install it in `~/.tokred/bin` (`%LOCALAPPDATA%\tokred\bin`), add it to `PATH` and run `tokred configure` and `tokred proxy enable`. `uninstall.sh` / `uninstall.ps1` undo it. Options: `--yes`, `--port`, `--version`, `--no-modify-path`, `--no-alias`, `--no-proxy`, `--no-configure`, and `TOKRED_HOME`, `TOKRED_RELEASE_BASE_URL`.
- The installer defines a `claude` wrapper (bash, zsh, fish, PowerShell) that runs `tokred initialize` first and then the real Claude Code; fail-open, skipped with `--no-alias` / `TOKRED_NO_ALIAS=1`, bypassed with `command claude`.
- `tokred proxy enable` asks for the proxy port (default 1029, 1024 to 65535, must be free) and whether conversation content may be recorded for the dashboard; new flags `--port`, `--record-content <true|false>` and `--yes` and variables `TOKRED_PORT`, `TOKRED_RECORD_CONTENT` (a flag wins). A quiet re-run never changes a choice you made, and a running proxy is restarted on a new port.
- `tokred uninstall`: stops the proxy, removes its user service, turns compression off, removes the hooks and agents and optionally the data folder.
- Tests for the install scripts (`tests/installer/`) in CI on Linux, macOS and Windows, and `shellcheck`.

### Changed

- No native Windows ARM build is published: the `ring` crate does not cross-compile for `aarch64-pc-windows-msvc` in CI. `install.ps1` installs the x86_64 build on Windows on ARM, which runs it through its emulation.

- `tokred uninstall` removes tokred completely and asks nothing: the proxy and its service, the hooks, agents and proxy URL in Claude Code, the data folder, and then the binary, the `PATH` entry and the `claude` wrapper (the uninstall script is embedded in the binary). If a step fails the binary is kept. `--yes` and `--purge` are accepted and ignored.
- `tokred unconfigure` removes only the hooks and agents; it no longer touches the proxy's `ANTHROPIC_BASE_URL` (only `tokred proxy enable` / `disable` and `tokred uninstall` do).
- `tokred configure` no longer sets up the proxy: it only touches hooks and agents (plus the database and the default `tokred.toml`), and writes no `[proxy]` value beyond the defaults (`enabled = false`). `tokred proxy enable` does it, with `--port` and `--record-content`; the install scripts run it after `configure`, and `--no-proxy` (`-NoProxy`, `TOKRED_NO_PROXY=1`) skips it. The `--proxy-port`, `--record-content` and `--no-record-content` flags of `configure` are gone, and a declined plan exits with code 2 (the install script then skips the proxy too).
- `tokred stats`, `tokred metrics` and the HTML report read the proxy's exact per-request data as their primary source and the transcripts for what the proxy did not see, merged so no request is counted twice ([metrics](docs/metrics.md)). Without the proxy nothing changes.
- Upgrading: installations configured before this have `enabled = false` written in `tokred.toml` and it is kept, so the proxy features stay off until you run `tokred proxy enable` once. An installed user service stops starting the proxy while it is disabled (`tokred proxy service uninstall` removes it).
- The `persist_base_url` key of `[proxy]` no longer exists (an old file that still has it keeps working): `enabled` now does what it was meant for.
- Distribution is the GitHub release only: the install scripts and the archives are release assets. Homebrew, Scoop, winget and crates.io publishing, `publish-package-managers.yml` and their secrets are gone.
- `tokred update` replaces the binary only when the install script put it in the install folder; any other copy is told to reinstall with the script. The daily version notice follows the same rule.

- `unconfigure` leaves settings it does not understand untouched instead of failing.

### Fixed

- `tokred unconfigure` now records that tokred is not configured, so the re-application that follows an upgrade no longer puts back the hooks and agents you removed (`tokred configure` turns them on again).

- `tokred uninstall` (and `tokred proxy status`) now ask the Windows scheduler whether the proxy's logon task exists; the task has no file, so it used to be skipped and left behind.

- On Windows, `tokred proxy start` run with its output captured (`out=$(tokred proxy start)`) no longer waits for the proxy to exit: the background proxy is started through `Start-Process`, which does not pass on the caller's pipes.
- Cache writes with the 1-hour lifetime are priced at their own rate (2x input) instead of the 5-minute rate (1.25x); `stats` previously under-reported them. Existing usage is backfilled automatically on the next `stats` run.
- The "latest model of a family" (used for the all-on-opus comparison and for unknown models) now ranks by version before date, so current undated ids such as `claude-opus-5-5` are no longer outranked by dated 4.x ids.
- Fable and Mythos are recognized as model families (price fallback, report colors).
- Transcript ingestion on Windows read stale file sizes while Claude Code still had the transcript open.

- Hook commands are recognized for any executable whose name starts with `tokred` (renamed or versioned binaries no longer cause duplicate entries).

### Added

- The proxy works behind a corporate network ([proxy](docs/proxy.md)): it tunnels through `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` with `CONNECT` (honouring `NO_PROXY`), and trusts the operating system's certificates and the PEM files of `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE` and the new optional `[proxy] ca_bundle`. Bad values are ignored with one warning. `tokred doctor` and `tokred proxy status` show the proxy host and the CA files.
- `tokred initialize`: makes Claude Code's `settings.json` agree with `[proxy] enabled`, for a `claude` shell wrapper to run before every launch. Enabled: starts the proxy if needed and sets `env.ANTHROPIC_BASE_URL` once it answers (removing it again, with a warning, if the proxy cannot start); disabled: removes tokred's URL. Another gateway is never overwritten. It is quiet, fast (one file read, one health probe) and always exits 0. `tokred proxy enable|disable` switch it and apply it at once; `[proxy] enabled` is now the single switch (`tokred claude` runs `claude` directly, `proxy start` refuses and `doctor` shows "disabled" while it is off), `configure` turns it on for a new installation, and `unconfigure` removes tokred's URL. The `session-start` hook starts a dead proxy when the session is pointed at it, and `doctor` warns about a URL that points at a proxy that is not running.
- Proxy request records now include the stop reason, the tool names offered, the cache lifetime asked for (`5m`/`1h`) and the routing mode in force (schema version 6).
- `[proxy] retention_days` is applied: data older than that is deleted when the proxy starts and every hour (`0` keeps everything).
- Live dashboard served by the proxy: `GET /dashboard` shows the active sessions and the last 10, and for each what was sent to Claude, what came back (streamed as it arrives) and the subagents running; `GET /metrics` serves the HTML report of `tokred metrics`. Real time through server-sent events; sign-in with a one-time token (`tokred dashboard [--no-open]`). See [dashboard](docs/dashboard.md).
- `[proxy] log_bodies` and `[proxy] retention_days` are now read: with `log_bodies = true` the dashboard also keeps message contents (only what is new in each request and the response, truncated; never headers or credentials), pruned to the active sessions and the last 10, and never older than `retention_days`. With it off (the default) nothing about contents is read or stored.
- Database migration v5: `captured_turns` (stop reason and tools of every turn; contents only with `log_bodies`).
- Optional local proxy between Claude Code and the Anthropic API, off until used ([proxy](docs/proxy.md)): exact measurement of every request (model, tokens including 1-hour cache writes, latency, cache-miss cause, proxy overhead); routing of subagent and auxiliary requests to a cheaper model (`observe` by default; the main loop's model is never changed); advice about the main loop (`tokred suggest`, `tokred statusline`, and an optional one-line message from the `route` hook); reversible compression of large tool results with the `tokred_retrieve` MCP server (`observe` by default). It is fail-open: if anything fails, the original request is forwarded unchanged.
- `tokred claude [args...]`: starts the proxy if needed and runs Claude Code through it for this run only, passing every argument through.
- `tokred proxy start|stop|status` and `tokred proxy service install|uninstall`; `tokred routing status|on|off|explain`; `tokred compression on|off|status` (`on` registers the MCP server in `~/.claude.json` with a `.tokred.bak` backup).
- `tokred doctor` shows the proxy (running, modes, where `ANTHROPIC_BASE_URL` points); `TOKRED_BENCH_RUNS` sets the runs of `doctor --bench`.
- With the proxy recording: `tokred stats --by origin` and `--by agent-type`, and a Proxy section in `stats`, `metrics` and the HTML report with the overhead against its 10 ms budget, routing decisions with observed and applied savings, the model asked for against the one that answered, prompt-cache misses by cause, and the latest pending suggestion (`stats --json` gets a `proxy` key).
- New `[proxy]`, `[routing]`, `[routing.tiers]`, `[suggest]`, `[compression]` and `[shadow]` sections in `tokred.toml` ([configuration](docs/configuration.md)).
- Reports, pricing, updates and hooks: colored terminal dashboards (`tokred stats`, `stats latency`, `metrics`) with an offline HTML report; price table with an embedded snapshot and a daily sync; the daily version check and `tokred update` (checksum-verified self-update for script installs); `tokred audit`; `tokred cache`; the six hooks (`guard-read`, `guard-search`, `route`, `enforce`, `session-check`, `session-start`) with latency recording and `doctor --bench`; the `dist` release pipeline with Homebrew, Scoop, winget and crates.io publishing.
- `TOKRED_OFFLINE`, `TOKRED_PRICES_URL`, `TOKRED_RELEASES_URL`, `NO_COLOR` and `CLICOLOR_FORCE`.
- Workspace bootstrap (ten crates, CI, cargo-deny, bilingual docs).
- `tokred configure`: detects the Claude Code folder, shows a plan, merges tokred hooks into `settings.json` without touching anything else, installs six subagents, creates the SQLite database and `tokred.toml`.
- `tokred unconfigure [--purge]`: removes only what tokred added.
- `tokred doctor [--json]`: reports hooks, agents, other hook tools and problems.
- `tokred hook <event>`: fail-open stub for the six hooks (the real behaviour arrives in later releases).
