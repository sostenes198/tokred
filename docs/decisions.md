**English** | [Português](decisions.pt-BR.md)

# Decision log

The non-obvious decisions behind tokred, with the reason for each. Format: date, context, decision, consequences. Where Claude Code's official documentation differs from an earlier assumption, the documentation wins and the divergence is recorded here. An entry that a later one replaces is cut down to one line and a pointer.

## 2026-10-28 — Subagent tool is `Agent`, matcher `Agent|Task`

- **Context:** the sub-agents documentation names the tool `Agent` (renamed from `Task` in Claude Code v2.1.63), with `Task(...)` still accepted as an alias.
- **Decision:** the `enforce` hook uses the matcher `Agent|Task` (the matcher syntax accepts `|`-separated exact names).
- **Consequences:** works on both old and new Claude Code versions.

## 2026-10-28 — Hook `timeout` is in seconds

- **Context:** the hooks documentation defines `timeout` in seconds (default 600 for command hooks).
- **Decision:** every tokred hook is registered with `timeout: 5`.

## 2026-10-28 — tokred hook commands are recognized structurally, not by the substring `tokred hook `

- **Context:** the entries tokred writes look like `"/opt/homebrew/bin/tokred" hook guard-read`: the closing quote sits between `tokred` and `hook`, so the substring `tokred hook ` never occurs in them. Matching on it would never find (and so never replace or remove) tokred's own entries.
- **Decision:** a command is tokred's when it runs an executable whose file name starts with `tokred` (`tokred`, `tokred.exe`, or a versioned or renamed copy such as `tokred-0.4.0`), optionally quoted, followed by ` hook <name>`. A unit test found this: under `cargo test` the binary is called `tokred-<hash>`, and an exact-name rule made `configure` add duplicates and `unconfigure` remove nothing.
- **Consequences:** idempotency and unconfigure work for quoted paths, Windows paths with spaces, renamed binaries and a bare `tokred hook x`. A wrapper such as `bash -c 'tokred hook x'` is deliberately not recognized.

## 2026-10-28 — Hooks never fail and the return-format labels are in English

- **Decision:** `tokred hook <event>` is dispatched **before** argument parsing and always exits 0, even for an unknown event, because Claude Code treats exit code 2 as "block". The agent return format uses the English labels `RESULT`, `FILES` and `PENDING`.

## 2026-10-28 — Minimum Rust version is 1.89

- **Context:** edition 2024 needs 1.85, but `rusqlite` 0.41 declares 1.89.
- **Decision:** `rust-version = "1.89"`, verified with `cargo +1.89 check --locked` locally and in CI.

## 2026-10-28 — Data folder from `BaseDirs`, not `ProjectDirs`

- **Context:** on Windows `ProjectDirs` appends `\data` (`%APPDATA%\tokred\data`), which is not the folder we want.
- **Decision:** use `BaseDirs::data_dir().join("tokred")`, which gives `~/.local/share/tokred`, `~/Library/Application Support/tokred` and `%APPDATA%\tokred`.

## 2026-10-28 — `option-ext` (MPL-2.0) allowed by a per-crate exception

- **Context:** `directories` pulls in `dirs-sys`, which pulls in `option-ext`, licensed MPL-2.0, and that is not in the licence allow-list (MIT, Apache-2.0, BSD, ISC, Unicode, Zlib).
- **Decision:** keep `directories` and add a `cargo-deny` exception for that one crate. MPL-2.0 is file-level copyleft and the crate is used unmodified. `deny.toml` also allows `CDLA-Permissive-2.0`, the Mozilla CA certificate data bundled by `webpki-roots`.

## 2026-10-28 — Details of `configure` / `unconfigure`

- `--claude-dir` is accepted by `configure`, `unconfigure` and `doctor`; an explicit but invalid path is an error and does not fall back to other candidates. `configure` also takes `--force`.
- A blank `settings.json` counts as `{}`; invalid JSON aborts with nothing written, no backup and no data folder created.
- `settings.json` is only rewritten when the merged result differs, and is re-serialized with 2-space indentation (key order is preserved, original whitespace is not). The backup holds the exact previous text.
- Round-trip limit: an event array or `hooks` object that was **already empty** and that tokred merged into is removed by `unconfigure`, because "emptied by our removal" cannot be told apart from "empty to begin with". Empty containers tokred did not touch are preserved. If `settings.json` did not exist, `unconfigure` leaves `{}` behind.
- `unconfigure` keeps agent files you edited (and, when the database is gone, every agent file, since their origin cannot be verified). It is lenient: it removes tokred's handlers and leaves anything it does not understand exactly as it is. Only `configure`, which has to write into those places, reports a bad shape.
- `--purge` refuses to delete a folder that has neither `tokred.db` nor `tokred.toml`.
- The database is created only after the plan is confirmed. `tokred.toml` is rewritten (without comments) only when keys were missing; otherwise it is left byte for byte.
- Terminal I/O goes through a small `Io` seam, and command output is rendered by pure functions into `String`s, so prompts, plan text and every `doctor` state are unit-tested without a terminal.

## 2026-10-28 — Response cache and artifact summaries: measured, not built

- **Decision:** an exact-match response cache and stored summaries of artifacts are not built; the proxy only measures how many requests are exact repeats (`[shadow] response_cache`). Agentic prompts rarely repeat, hooks cannot intercept API calls, and Anthropic already caches the prompt prefix server-side. See the proxy entries below for what was built instead.

## 2026-10-28 — Hooks: where the official documentation decides

Confirmed against <https://code.claude.com/docs/en/hooks>:

- `warn` mode emits `additionalContext` only. It never emits `permissionDecision: "allow"`, because that would skip the user's own permission prompt.
- `Stop` emits `systemMessage` only (a message for the user): `additionalContext` or `decision` on `Stop` would make Claude continue. `SessionStart` prints nothing, because its stdout becomes model context.
- `SessionStart` `source` has a fifth value, `fork`; like `startup` and `resume` it is not a cut (only `clear` and `compact` are).
- File reads are tracked per `session/agent_id`, because a subagent's context is separate from its parent's. A read in the same second as a cut is allowed (strict `>`).
- Deny globs match the path relative to the session's `cwd`. Images and PDFs are exempt from the size check. A `Read` with any argument besides `file_path`, `offset` and `limit` (such as `pages`) skips the re-read check. Bytes avoided by a ranged read are estimated at 80 bytes per line, and files above 16 MiB are not hashed.
- `guard-search` counts files per repository root (cached in the tiered cache, keyed by root, threshold and root mtime), stops counting at threshold + 1 and gives up after 100 ms, in which case it allows the search.
- `route` stores only actionable classes in `classifications` (never `other`), caches classifications by `blake3(prompt)` plus a rules version, and behaves like `warn` in `enforce` mode. For `enforce`, architect and planner are interchangeable.
- `session-check` warns once per session, again after the context grows by half the limit, or after a clear or compact. A literal `~/` in `transcript_path` is not expanded.
- stdout cannot be closed (`unsafe` is forbidden): the answer is flushed and the database row is written last, but Claude Code sees the end only when the process exits. A hook never creates the data folder or `tokred.db`; an invalid `tokred.toml` means defaults. A watchdog (`TOKRED_HOOK_GUARD_MS`, default 2 s) turns a stuck hook into a `timeout_guard` outcome.

## 2026-10-28 — Pricing, update and the self-update path

- The price snapshot keeps LiteLLM's own shape with normalized keys, so one parser reads both the live list and the embedded file (regenerated by `scripts/update-price-snapshot.sh`). Key preference: bare Anthropic keys, then `anthropic/`, `vertex_ai/`, `bedrock/`, then Bedrock-native `anthropic.`; regional and reseller keys are dropped. Missing cache prices are derived (write 1.25x, read 0.1x of input).
- "Latest price of a family" is the entry with the highest version, then the most recent date, in its model id (see the 1-hour cache entry below).
- A sync that returns no rows never replaces the table. `price_sync_attempt_at` is recorded so a failed sync is not retried on every command.
- `BinarySwap` is a port owned by `tokred-update` (the `FileSystem` port has no rename or chmod). Self-update verifies the SHA-256 sidecar before touching anything and replaces the binary atomically; on Windows it renames the old `.exe` to `.old` and removes it on the next run. Linux always maps to the musl build. Release tags are `v<version>`.
- After a self-update the configuration is re-applied by running the **new** binary (`configure --yes`), so the new agent templates are the ones installed.
- `TOKRED_OFFLINE`, `TOKRED_PRICES_URL` and `TOKRED_RELEASES_URL`: one switch for every network call, and URL overrides for mirrors and for tests against a local server.

## 2026-10-28 — Metrics calculations

- The "everything on opus" counterfactual uses the **latest** opus price in the table. When the model actually used is a more expensive, older opus, the counterfactual can be lower than the real cost; the routing saving is then negative, which is accurate.
- A guard "block" is a `guard-*` event with decision `deny`; `warn` saves nothing. Guard savings are always labelled an estimate. The session's model for pricing them is the one with the most messages (ties: smaller id; fallback opus).
- The projection divisor is `min(window, days since the first row)`, at least 1. `TimeRange::last` is `[now - span, now + 1)`.
- A file replaced by a larger one is not detected by ingestion because there is no content hash; a shrunk file restarts from zero.
- `route` classifications are counted from `route` hook events, so there is no separate classification table to query for the report.

## 2026-10-28 — Audit: locations confirmed in the official documentation

- MCP servers are not configured in `settings.json`. User scope is the top-level `mcpServers` of `~/.claude.json`, local scope is `projects["<path>"].mcpServers` in the same file, project scope is `<project>/.mcp.json`; precedence is local, then project, then user. `settings.json` only contributes `disabledMcpjsonServers`. Whether `.claude.json` moves with `CLAUDE_CONFIG_DIR` is not documented, so `<claude>/.claude.json` is tried first, then `~/.claude.json`.
- Project memory also includes `.claude/CLAUDE.md`, `CLAUDE.local.md`, ancestor directories and always-on `.claude/rules` files. Imports follow the documented 4-hop limit and skip fences, code spans and block HTML comments.
- The advice is "move to path-scoped rules or a skill", not "split with `@import`", because imports do not reduce context cost. Skill bodies load only on invocation, so skills also report their start-up weight. Thresholds: 200 lines for `CLAUDE.md` and 500 lines for skills (both from the documentation), and 2000 tokens for agents (a tokred heuristic).
- Plugin skills, agents and servers, `commands/*.md`, claude.ai connectors and managed servers are not enumerated; usage seen in transcripts for servers not configured locally is listed separately.

## 2026-10-28 — Agent templates live in the install crate

- The agent templates live in `crates/tokred-install/templates/` because `include_str!` from the repository root would break a published crate.

## 2026-10-28 — Terminal dashboards, background chores and the benchmark

- `tokred stats`, `stats latency` and `metrics` print a colored dashboard in the terminal (the HTML report is the same data, not a replacement). Color follows `NO_COLOR` (always wins), `CLICOLOR_FORCE`, a dumb `TERM`, and whether stdout is a terminal.
- Notices from background chores go to **stderr** so `--json` output stays clean. Chores run only after commands that are not hooks, are best effort, and never fail the command: the daily price sync, the daily version check and the quiet re-application of the configuration after an upgrade.
- `doctor --bench` runs each hook as a separate process in a scratch data folder with its own database (so the hooks do real work and the user's data is untouched), times it from spawn to exit, and stores the difference from the hooks' own measurements as the process start-up overhead in `meta`. `TOKRED_BENCH_RUNS` overrides the default of 50 runs per payload.
- Line coverage is read from the per-line views (`lcov`, annotated text), not from LLVM's summary, which counts macro regions and `?` error branches separately and can read a fraction of a percent lower.

## 2026-10-28 — The proxy, and a model policy that respects the user's choice

- **Context:** the official gateway guide, the prompt-caching page and the preserved-thinking page showed that (a) Claude Code labels requests (`x-claude-code-request-class`, agent type, agent id, prompt id, session id), (b) gateways are told to inspect without modifying and a rewritten history can trigger a "bound to a different conversation" error, (c) changing the main model mid-session rebuilds the whole prompt cache, and (d) auxiliary requests may run on the main model and some reuse the conversation's cache.
- **Decision (agreed with the author):** the proxy classifies requests and may choose the model **only** for subagents and auxiliary requests. The **main loop keeps the user's configured model**; tokred only **summarizes and suggests** a cheaper one when the task was simple, preferably when the cache is already cold. Compression and the response cache are measurement-only until the data justifies more. Every behaviour that alters traffic starts in `observe` mode.

## 2026-10-28 — Proxy probe, first results (API-key login)

Run with a throwaway pass-through proxy (not kept in the repository; its findings are this entry) and the real Claude Code 2.1.312 in print mode, using an API-key login.

- **Pass-through works.** Streaming (SSE) is forwarded unchanged and `usage` is readable from `message_start` and `message_delta`. The response `model` matches the request.
- **Headers actually sent:** `x-claude-code-session-id` on every request and `x-claude-code-agent-id` on subagent requests only. `x-claude-code-request-class`, `x-claude-code-agent-type` and `x-claude-code-prompt-id`, which the gateway guide says are sent, did **not** appear in this version and mode, so classification cannot depend on them. Main versus subagent is told apart by the presence of `agent-id`; the kind of subagent by its tool set (the Explore subagent had 10 read-only tools against 15 for the main loop) and system blocks.
- **Subagents build their own cache** (the first turn wrote about 19.9k tokens, the next turns read them back), which is why a subagent's model is decided once and kept.
- **Rewriting the model of subagent requests works.** With the main loop on Sonnet, rewriting only the subagent's requests to Haiku was accepted by the API on all three turns (including tool results sent back), Claude Code showed no problem with a response from a different model, and the final answer was correct. The main loop's requests were never touched.
- **Decision:** the main loop's model is **never** changed, with no configuration option to do so; tokred only advises.

## 2026-10-28 — Proxy probe, subscription result (claude.ai login)

Same probe and Claude Code version, on a claude.ai **subscription** login (`ANTHROPIC_API_KEY` unset), through `ANTHROPIC_BASE_URL` only.

- **Pass-through works on a subscription.** Status 200 with streaming. The credential travels in the `authorization` header (OAuth), not `x-api-key`, and the `oauth-2025-04-20` value of `anthropic-beta` is present, so a proxy must forward `authorization` and the whole `anthropic-beta` header untouched, as the gateway guide says. The request also carried `extended-cache-ttl-2025-04-11` (the 1-hour cache).
- **The headers are the same as with an API key.**
- **Rewriting a subagent's model works on a subscription** (Sonnet to Haiku, status 200 each time, correct answer).
- **Not covered by testing:** Anthropic's usage terms for subscription traffic through a proxy (see the entry below), per-model plan limits, and very long sessions.
- **Consequence:** technically go for both login types. The proxy stays opt-in.

## 2026-10-28 — Proxy probe, interactive mode: how to recognize auxiliary requests

Interactive `claude` (2.1.312, API-key login) through the probe, two short prompts.

- **`x-claude-code-request-class` is not sent in interactive mode either.** Only `x-claude-code-session-id` arrived (and `x-claude-code-agent-id` for subagents).
- **The auxiliary request is recognizable by its shape.** Besides the main-loop turns (16 tools, adaptive thinking), one request had **no tools, no thinking and exactly one message** (most likely the session title). It ran on the main model and read and wrote **no** prompt cache, so moving it to a cheaper model loses nothing.
- **Startup checks:** twelve requests with `max_tokens: 1`, no tools and no system prompt returned `429` and were retried by Claude Code; the proxy must (and does) pass `429` through unchanged.
- **Decision:** a request is auxiliary only when it has no `agent-id`, no tools, no `thinking` and exactly one message (or when a future Claude Code sends `x-claude-code-request-class: auxiliary`). The main loop always carries tools, so it can never match. Everything else without an `agent-id` is the main loop and is never rewritten.

## 2026-10-28 — 1-hour cache writes and model family ranking

- **1-hour cache writes.** Current transcripts split `cache_creation_input_tokens` into `cache_creation.ephemeral_5m_input_tokens` and `ephemeral_1h_input_tokens`, and on the probe machine every write was 1-hour. LiteLLM publishes the price as `cache_creation_input_token_cost_above_1hr`. `Usage` gained `cache_write_1h` (a part of `cache_write`) and `ModelPrice` gained `cache_write_1h`; when the price list lacks it, the documented rate of 2x input is used. Migration v3 adds the columns and clears the ingestion offsets so existing rows are backfilled without duplicates.
- **Family ranking.** Versions rank before dates (`claude-opus-5-5` > `claude-opus-4-7-20260416`). The earlier rule, which ranked undated ids below dated ones, made the counterfactual price everything at an older model.
- **Fable and Mythos** are families. The `Family` type stays a closed enum (unknown families still resolve to "unpriced"), which keeps exhaustive matching.

## 2026-10-28 — Subscription traffic through the proxy: what the terms say

Checked against Anthropic's Consumer Terms (Section 3, item 7) and the Claude Code gateway documentation. This is a reading of the published text, not legal advice.

- The Consumer Terms forbid accessing the Services "through automated or non-human means" except with an API key "or where we otherwise explicitly permit it". They do not mention local proxies or gateways by name.
- The Claude Code documentation explicitly describes the configuration tokred uses: with only `ANTHROPIC_BASE_URL` set, "a saved claude.ai login remains the active credential" and gateways "must forward the OAuth capability in `anthropic-beta`". **Measuring** subscription traffic through the local proxy is therefore a documented setup.
- **Rewriting the model** of subscription requests is not addressed by either text. It lowers usage rather than bypassing limits, but it is not explicitly permitted. Decision: routing stays in `observe` by default for every login, and `doctor` prints a note whenever `enforce` is on, so turning it on is an informed choice of the user.

## 2026-10-28 — The proxy asks the upstream for `Accept-Encoding: identity`

- **Context:** the real API gzips its responses when the client allows it, and the proxy has to read `usage` from the response body to measure tokens exactly.
- **Decision:** the proxy rewrites `Accept-Encoding` to `identity` on the request it sends upstream. It is the only request header it rewrites; every other header is forwarded as it came.
- **Consequences:** responses arrive uncompressed, so streaming and `usage` can be read as they pass. If an upstream compresses anyway, the response is forwarded unchanged and its usage is not recorded (fail-open: Claude Code is never affected, the measurement has a gap).

## 2026-10-28 — Wiring of the background proxy

- **Context:** `tokred proxy start` has to leave a proxy running after the command returns, and `proxy stop` and `tokred claude` have to find it and talk to it safely.
- **Decision:** the background proxy is the same binary, started as `tokred proxy start --foreground` (the user service runs the same command), so there is no second executable to keep in sync. State lives in `<data>/proxy/`: `secret` (owner-only, `0600` on Unix), which the admin endpoints (`/__tokred/...`) require, and `pid`. `proxy stop` sends the shutdown request and **waits until the process is gone** (it fails if the proxy did not stop) before clearing the pid file.
- **Consequences:** `proxy stop && proxy start` is safe to script; a stale pid file is ignored when the proxy does not answer its health endpoint.

## 2026-10-28 — The compression circuit breaker survives a restart

- **Context:** when rewritten requests keep being rejected, the breaker turns compression off. In memory only, a restart would turn it on again and the same failures would repeat.
- **Decision:** the trip is persisted in the database `meta` table (`compression_tripped_at`). While it is set, the proxy starts with compression in `observe` even if `tokred.toml` says `enforce`; `tokred compression on` clears it.
- **Consequences:** a restart (or a reboot with the user service) never silently re-enables a feature that was failing; `tokred compression status` shows that the breaker tripped.

## 2026-10-28 — `tokred compression on|off` edits Claude Code's user config only when run

- **Context:** reversible compression needs the `tokred_retrieve` MCP server registered in Claude Code, which means editing its user config (`~/.claude.json`), a file that holds more than tokred's data.
- **Decision:** only `tokred compression on` (registers the server and sets `[compression] mode = "enforce"`) and `off` (unregisters it and sets `observe`) touch that file, and only when you run them. The previous content is saved next to it as `.tokred.bak` first. `configure`, `tokred claude` and the proxy never edit it.

## 2026-10-28 — Known risks of the proxy

- **Compaction can look like a simple prompt.** tokred recognises a compaction request only by the `x-claude-code-compaction` and `x-claude-code-request-class` headers, and the Claude Code version tested sent neither, so one could be misread as a simple prompt and feed a cheaper-model suggestion. Suggestions only advise, and routing never rewrites a compaction request, so the cost of a wrong reading is a wrong suggestion.
- **Usage is not read when the upstream compresses anyway** (see the `Accept-Encoding` entry above).

## 2026-10-28 — Windows: a captured `proxy start` waits for the proxy

- **Context:** the end-to-end tests of the proxy hung on Windows only: a detached proxy inherits the pipe handles of a caller that captures output, so the caller never sees the end of the output until the proxy exits.
- **Decision:** the proxy is started through `Start-Process` on Windows (see the 2026-10-29 entry below). CI jobs stop after 20 minutes so a hang fails with its log.

## 2026-10-29 — Corporate networks: HTTP proxy tunnel and extra certificate authorities

- **Context:** the proxy connected straight to the API with the bundled Mozilla roots, so it failed behind a corporate HTTP proxy and behind one that inspects TLS with its own authority.
- **Decision:** the upstream client tunnels with `CONNECT` through the proxy named by `HTTPS_PROXY`, else `ALL_PROXY`, else `HTTP_PROXY` (and the lower-case names; `HTTP_PROXY` is a fallback because many corporate images only set that one), honouring `NO_PROXY` with the curl rules. This uses `hyper-util`'s `Tunnel` and proxy matcher (the `client-proxy` feature; no hand-written handshake). Only `http://` proxies are supported and only for an `https` upstream (a loopback `http` upstream is always direct). Trust is the bundled roots plus the operating system's store (`rustls-native-certs`, `ring` only) plus the PEM files of `[proxy] ca_bundle`, `NODE_EXTRA_CA_CERTS` and `SSL_CERT_FILE`, parsed with `rustls-pki-types` (not the unmaintained `rustls-pemfile`). Unsupported (`socks5://`, `https://`) or malformed proxy values, and missing or empty CA files, are logged once by variable or file name (never the value, which can hold a password) and ignored: the proxy connects directly or with the remaining roots, so tokred never breaks Claude Code. A reachable proxy that refuses the tunnel, or an untrusted certificate, gives the usual `502`. The settings are read once at start, from the environment of the process that starts the proxy.
- **Consequences:** TLS-inspecting and tunnelling networks work with the variables Claude Code already uses. New dependencies: `rustls`, `rustls-native-certs`, `rustls-pki-types`, `webpki-roots` and, for tests only, `rcgen` and `tokio-rustls`; `cargo deny` passes and there is no `aws-lc` (it needs `cmake` and NASM and breaks the musl and Windows builds). SOCKS and TLS to the proxy stay out of scope. Environment changes need a proxy restart.

## 2026-10-29 — Windows: the background proxy starts through `Start-Process`

- **Context:** the detached proxy inherited the pipes of a capturing caller (see the 2026-10-28 entry).
- **Decision:** on Windows `spawn_detached` runs `powershell.exe -NoProfile -NonInteractive -Command "Start-Process -FilePath <exe> -ArgumentList <args> -WindowStyle Hidden -PassThru | Select-Object -ExpandProperty Id"`, which starts the proxy without this process's handles, and reads the pid from the first line of its output (only the first line, so a stray open pipe cannot hold the call up). Paths and arguments are quoted by pure functions tested on every operating system; a plain detached spawn is the fallback when PowerShell cannot be used. No `unsafe` and no new dependency. The end-to-end test that runs `proxy start` with captured output runs on every operating system and waits for the command on a thread with a 60-second deadline, because killing the command does not help while the proxy still holds the captured pipes.
- **Consequences:** `out=$(tokred proxy start)` returns on Windows too. On Windows the `session-start` safety net and `initialize` still start the proxy detached with no standard streams, and a child can inherit handles such as the pipe a hook's output is read from, so the caller may wait for its own timeout. That is kept: waiting is better than Claude Code being locked out by a dead proxy, and the hook still exits 0.

## 2026-10-29 — The proxy's requests and the transcripts are merged once, before any metric

- **Context:** with the proxy running every request is in `proxy_requests` and in the transcripts (`usage`); adding the two counts each twice, and using only one loses either the exact data (model that answered, 1-hour cache split, latency) or the requests made while the proxy was off.
- **Decision:** `merge_usage` (pure, in `tokred-metrics`) runs once, where the data is loaded, so `stats`, the terminal dashboards, the HTML report, the projection and the before/after read one list. A proxy request is paired with the transcript row of the same `message_key`; only a proxy request **without** a key falls back to the same session, equal tokens (the four counters, not the 1-hour split) and a time within 5 seconds of the request or up to 5 seconds after the end of its response (the transcript line is written when the response ends, so a plain "within 5 s of the arrival" would miss any answer longer than 5 s). Pairing is one to one and nearest in time first, so two identical requests in the same seconds are never collapsed. A pair takes the proxy's numbers and the transcript's project and key; unpaired rows of both kinds stay. A request answered with a non-2xx status or without tokens is not a message. Two proxy rows with the same id, or the same message key, count once. `usage` rows gain the optional `origin` and `agent_type` (never stored) so `--by origin` and `--by agent-type` need no second data path; without the proxy `origin` falls back to `main` or `subagent` from the transcripts.
- **Consequences:** a proxied request is counted once whatever the order of ingestion; the routing saving prices the model that really answered; with no proxy rows the transcript rows pass through untouched. A key present on both sides but different is two requests (never merged by time). Covered by unit tests, a snapshot and properties (nothing lost, nothing repeated, merging again changes nothing).

## 2026-10-29 — `stats` shows the pending suggestion and never marks it shown

- **Context:** the status line deliberately never marks a suggestion as shown (it is redrawn constantly), so it does not hide it from the one-line message; `stats --json` must also not write to the database.
- **Decision:** `stats` shows the latest suggestion of the period that is not shown yet, with a pointer to `tokred suggest`, and does not change the flag; only the message channel sets it.
- **Consequences:** `stats` is read-only and repeatable; the same suggestion stays in `stats` until the message channel shows it.

## 2026-10-29 — The proxy sections of the HTML report appear only when the proxy recorded something

- **Decision:** the proxy sections (model routing, overhead, cache misses by cause, cost by origin and agent type) are rendered only when the report has proxy data, and the data is embedded in the page only then. Inside them an empty part (no cache misses, no decisions) is left out rather than shown empty.
- **Consequences:** a report without the proxy is byte for byte what it was before; the "no data yet" notice applies to the original sections.

## 2026-10-29 — One function builds the report from the repositories

- **Context:** the proxy serves the same report at `/metrics`, and `tokred-proxy` may not depend on the store or the report crates.
- **Decision:** `report_html(repos, prices, now, since, audit)` in `tokred-cli` takes the repository ports (usage, proxy requests, hook events, routing, suggestions, meta) and returns the HTML as a string: no transcript ingestion, no file or network access, no price sync. `tokred metrics` ingests the transcripts, then writes what `report_html` returns; the proxy gets the same function through the `DashboardSource` port of `tokred-core`.
- **Consequences:** the file and the page cannot drift apart.

## 2026-10-29 — Richer proxy request records

- **Context:** `proxy_requests` needs the stop reason, the tool names, the cache lifetime and the routing mode a request ran under.
- **Decision:** four columns (`stop_reason`, `tools` as JSON text with names only and at most 200, `cache_ttl` as `5m`, `1h` or null, `routing_mode`); old rows keep null or empty values. `stop_reason` and the tool names were already parsed, so nothing new is parsed on the request path. `cache_ttl` is computed after the response from the body that was sent, looking only at the places Claude Code puts breakpoints (top level, `system`, `tools`, message content blocks); a `cache_control` without `ttl` counts as `5m`, and the longest one wins. `routing_mode` comes from the `[routing]` mode the proxy was assembled with, not from the hook notes: the notes are reset when a hook fails, which are exactly the requests worth explaining.
- **Consequences:** `tokred-store` still needs no JSON library: the tool names are serialized in `tokred-core`.

## 2026-10-29 — `retention_days` prunes every proxy table, on a schedule

- **Context:** `[proxy] retention_days` only means something if the proxy deletes old data itself.
- **Decision:** when the proxy starts, and then every hour, rows older than `retention_days` are deleted from `proxy_requests`, `routing_decisions`, `prompt_summaries`, `suggestions`, `shadow_savings`, `stored_results` and `compressed_blocks`, on a thread of its own (database work never runs on the request path). `0` keeps every row, and that rule lives in the pruning function because `prune_proxy_requests(now, 0)` on its own means "delete everything before now". The kept tool results and the replacements of a conversation that still has a request inside the period are kept whatever their age: a replaced tool result must be sent as the same bytes every turn (changing it breaks the prompt cache) and the model may still ask for the original. Records of a request in flight are never at risk: they are written when the response ends, with the time the request arrived.
- **Consequences:** a long-lived conversation keeps its compression data longer than `retention_days`; the cost is bounded by how many conversations are active.

## 2026-10-29 — `[proxy] enabled` is the single switch; `tokred initialize` reconciles Claude Code's settings

- **Context:** with the proxy URL persisted in Claude Code's settings and the proxy down, Claude Code cannot reach the API. The installer puts a `claude` shell wrapper in place that can run a command before every launch.
- **Decision:** `enabled` is the only user-facing switch. `false`: `tokred claude` runs `claude` directly (one line on stderr), `tokred initialize` removes tokred's URL from `settings.json`, `proxy start` and `proxy service install` refuse, `doctor` shows "disabled"; `proxy stop` and `status` still work and a running proxy is never stopped. `true`: `initialize` starts the proxy when it is not running and, once it answers, sets `env.ANTHROPIC_BASE_URL`; if the proxy cannot be started the URL is removed again (it is only written after the proxy answers, so there is no window with a dead URL) and a warning is printed. `tokred proxy enable` and `disable` edit `tokred.toml` and run the same reconcile at once; `unconfigure` leaves `tokred.toml` alone.
- **No `persist_base_url` option.** With `enabled` keeping the URL in `settings.json`, launches from an IDE that never run the wrapper already use it, so a second option would only be a second way to get the same state. A `tokred.toml` that still has the key keeps working (it is ignored). The risk is covered by three safeguards that hang on `enabled`: `initialize` never leaves a URL pointing at a proxy it could not start; the `session-start` hook starts a dead proxy; `doctor` warns.
- **A value that is not tokred's is never touched.** "Tokred's URL" is the current proxy URL or a URL tokred wrote before, recorded one per line in `<data>/proxy/base_urls` (a plain file, so the per-launch path does not open the database, and so a changed port is followed). The value exported in the shell counts as a current value too. Another gateway, even on `127.0.0.1`, makes `initialize` print one warning and do nothing (and start nothing). The file edit goes through `tokred-install` (pure merge functions with fixtures and property tests): it changes only that key, keeps its position and the other `env` keys, writes nothing when nothing changes and makes no backup (the backup `configure` made must keep the user's original).
- **`tokred initialize` is fail-open and quiet.** It always exits 0 (invalid arguments aside), prints nothing unless there is a warning (`--verbose` and `--json` say more), reads one file and probes the proxy once, and does not open the database. It settles on the Claude Code folder `claude` itself would use: `$CLAUDE_CONFIG_DIR`, else the folder saved by `configure`, else `~/.claude`.
- **The `session-start` safety net** is composed in the binary, after the hook has answered, and not in `tokred-hooks` (which has no network code): when `enabled` is true, the session's `ANTHROPIC_BASE_URL` (environment, else `settings.json`) is tokred's and the proxy does not answer a 200 ms probe, it starts the proxy detached and returns without waiting. It is gated on `enabled`, so the hook benchmark and the hook tests never start a proxy.
- **The user service.** While disabled, `proxy start --foreground` refuses, so an installed service would fail and be restarted by the service manager: `proxy service install` refuses while disabled and `proxy disable` says to run `proxy service uninstall` when a service is installed.
- **Consequences:** `tokred proxy disable` is the way out of any trouble, and so is deleting the key by hand.

## 2026-10-29 — Installation: scripts and release assets, no package managers

- **Context:** package managers (Homebrew, Scoop, winget, crates.io) each need their own credentials, repository or manifest to maintain, which is not worth it for a single binary.
- **Decision:** the GitHub release is the only channel. `install/install.sh` and `install/install.ps1` download `tokred-<target>` and its `.sha256` from the release (`releases/latest/download/`, or `releases/download/v$TOKRED_VERSION/`; `TOKRED_RELEASE_BASE_URL` overrides the base and accepts `file://`, which is what makes the scripts testable offline), verify the digest, install into `$TOKRED_HOME/bin` (`~/.tokred/bin`, Windows `%LOCALAPPDATA%\tokred\bin`), add that folder to `PATH` and run `tokred configure`. `dist-workspace.toml` has `installers = []` and an `extra-artifacts` entry that uploads the four scripts next to the archives; the `Release` workflow needs no secret beyond `GITHUB_TOKEN` (cutting a release from Actions has its own, see [release](release.md)). The same scripts are on `main` for use before a release exists.
- **Design:** the scripts are thin. Everything interactive (port, content recording, confirmations) is Rust, unit-tested through the `Io` seam; the scripts only fetch, verify, copy, edit `PATH` and profiles, and call the binary. With `curl | sh` stdin is the pipe, so the script gives the binary the terminal (`</dev/tty`) when there is one and `--yes` otherwise. No `sudo`, nothing written outside `TOKRED_HOME` and the profile files, root refused without `--yes`. The shell script's own downloads use `--proto '=https'` for `https://` and a plain copy for `file://`. The `.sh` files are LF on every checkout (`.gitattributes`).
- **PowerShell caveats:** `irm | iex` cannot pass parameters, so each option also has an environment variable. The user `PATH` is changed with `[Environment]::SetEnvironmentVariable(..., 'User')`, which expands `%VARS%` of existing entries when it writes them back; the simpler call was accepted. The scripts keep to ASCII, `throw` instead of `exit` (which would close an interactive window under `iex`) and rename a running `tokred.exe` to `.old` instead of overwriting it. They are exercised by `install_test.ps1` in the Windows CI job.
- **Consequences:** installing and configuring are one step: the script runs `tokred configure`, then `tokred proxy enable` (see the `configure` entry below). `tokred update` replaces only a binary the script installed (see below).

## 2026-10-29 — `tokred update` only replaces the script's own binary

- **Decision:** `InstallMethod` is `Script | Other`. A binary whose folder equals `$TOKRED_HOME/bin` (resolved with `canonicalize` on both sides, so macOS `/var` versus `/private/var` does not matter) updates itself with the existing download, checksum and atomic swap (a `.exe` on Windows is renamed to `.old` first); anything else (built from source, copied by hand) gets the notice "reinstall with the install script". The daily version check is unchanged and uses the same hint.
- **Consequence:** a binary in an unknown location is never touched. That is the safer failure: tokred never overwrites a binary it did not put there.

## 2026-10-29 — The `claude` wrapper

- **Context:** the proxy switch has to take effect in every terminal, and Claude Code reads `ANTHROPIC_BASE_URL` only at start. `tokred initialize` reconciles Claude Code's settings with the switch; something has to run it before `claude`.
- **Decision:** the install script defines a shell **function** `claude` in the same marked block as the `PATH` line: `claude() { if command -v tokred >/dev/null 2>&1; then tokred initialize; fi; command claude "$@"; }`. A function (not an alias) passes arguments and cannot recurse; `command claude` is the bypass. It is fail-open: without `tokred`, `claude` starts as before. The block is the same text in every POSIX profile (`~/.profile`, `~/.bashrc`, `~/.bash_profile`, `~/.zshrc`, and a `.zshrc` is created when the login shell is zsh), preceded by `unalias claude` because Claude Code's own installer may add an alias to `.zshrc`, which would make the function definition a parse error in zsh. Fish gets its own file, `conf.d/tokred.fish`, with the equivalent function (deleted on uninstall only when it holds nothing but our block); Windows gets a marked block in the `profile.ps1` of Windows PowerShell and, when present, pwsh, calling the real binary through `Get-Command claude -CommandType Application`. `cmd.exe` has no persistent aliases and is not covered.
- **Switches:** `--no-alias` / `TOKRED_NO_ALIAS=1` skip it and are independent of `--no-modify-path`; the block is rewritten on every run (so an older block gains the wrapper), never skipped when present, and written with `cat >` so symlinked dotfiles stay symlinks.
- **Limit:** if the real `claude` is reachable only through an alias, `command claude` cannot find it.

## 2026-10-29 — The dashboard: privacy trade-off, per-turn deltas, sign-in and shutdown

- **Context:** the proxy serves a live dashboard (`/dashboard`, `/metrics`) that shows what was sent to Claude and what came back. Showing that means message contents are stored, and Claude Code re-sends the whole conversation with every request.
- **Decision (privacy):** contents are captured only with `[proxy] log_bodies = true`, off by default. Off means nothing about contents is read from requests or responses, held in memory, sent to the browser or stored (the `turn_delta` event is content too and is not sent); the dashboard still shows metadata (models, tokens, cost, timings, status, stop reason, tool names, routing, subagents). On means per-turn **deltas**, not bodies: the messages new since the previous request of the same session and subagent (tracked by a hash per message, with a fast check of the first and last message; after a compaction or rewind the messages that differ are sent) and the reassembled response, each text truncated to 20,000 characters. Headers and credentials are never captured. Contents live in `captured_turns` and are pruned at start-up and hourly: only sessions that are active or among the last 10 keep them, and nothing outlives `retention_days`. The stop reason and the names of the tools called are metadata, so `captured_turns` has a row for every turn and only the content columns depend on `log_bodies`; pruning empties the contents and keeps the row until `retention_days`.
- **Decision (sign-in):** other local users can reach loopback and a browser cannot send `x-tokred-secret`, so `POST /__tokred/dashboard-token` (with the secret) issues a token valid for 60 seconds and once; `GET /dashboard?token=` trades it for a random cookie (`HttpOnly`, `SameSite=Strict`, `Path=/` so `/metrics` shares it) and redirects. Tokens and cookies are compared in constant time against every stored value, and live in memory (a restart signs you out). Randomness comes from `RandomState`, like the admin secret, to avoid a new dependency. Requests with `Sec-Fetch-Site: cross-site` or `same-site` are refused (`same-site` too: another local web app on a different port is the same site), no CORS headers are sent, and every page carries a strict CSP. `/metrics` has its own CSP (`style-src 'unsafe-inline'`) and allows exactly the report's own inline scripts by SHA-256 hash, so the period toggle and tooltips keep working while nothing else can run.
- **Decision (events and merging):** a bounded broadcast channel of already-framed events; nothing is built when nobody listens, a slow subscriber loses the oldest events (it re-reads the API on reconnect; deltas are never replayed). The proxy keeps the turns in flight and the last ended ones in memory and merges them by request id over what the store returns, the live version winning, so a session shows both. Running subagents are tracked from the agent id of in-flight requests (running while one is in flight and for 60 seconds after the last); a sweep task ends them and stops with the proxy.
- **Decision (shutdown):** hyper waits for an in-flight streaming body when a connection is asked to stop, so an open event stream would hold the proxy until the drain timeout (10 s, longer than `proxy stop` waits). The event stream task selects on the shutdown signal, so open streams end at once; an end-to-end test opens `/dashboard/events`, runs `proxy stop` and checks that the stream ends and the process exits.
- **Consequences:** turning `log_bodies` on stores conversation text in the local database; the data never leaves the machine and is pruned, but anyone who can read the database can read it. A compaction request is shown as `auxiliary`. Only the newest 500 turns of a session are returned (`truncated: true`). Requests without a session id are not followed. The `/metrics` report is generated from the transcripts on each request, in a blocking task, without the project audit.

## 2026-10-29 — `uninstall` removes everything and asks nothing; `unconfigure` does not touch the proxy

- **Context:** an uninstall that asks questions, keeps the data folder or leaves the binary, the `PATH` entry and the `claude` wrapper to another script is not a removal. And `unconfigure` must not remove the proxy's `ANTHROPIC_BASE_URL`: that setting belongs to the proxy (the `claude` wrapper would put it back on the next launch, and a proxy meant to keep running would lose its place in the path).
- **Decision:** `tokred uninstall` is the complete removal and asks nothing: it runs, in order, `proxy stop`, `proxy service uninstall` (only when the service file exists for this home folder, so a sandboxed run never reaches the real `launchctl` or `systemctl`), `compression off` (only when the MCP server is registered), removes tokred's proxy URL from Claude Code's settings, `unconfigure`, deletes the data folder, then the binary, its folder, the `PATH` entry and the wrapper. Every step is best effort (a failure is printed and the next step runs, so a broken proxy never leaves hooks behind), except that if any step fails the binary is kept, so Claude Code never points at a binary that no longer exists. The last part is the uninstall script embedded in the binary (`--cleanup-only`), so the logic exists once and the installer tests of CI cover it on every system; on Windows it is started detached and waits for the process to exit, because a running executable cannot delete itself. The binary is deleted only when the install script put it in its folder. `unconfigure` removes only hooks and agents; only `tokred proxy enable` / `disable` and `uninstall` change the URL.
- **Consequences:** `--yes` and `--purge` of `uninstall` are accepted and ignored; the uninstall scripts take no options. The end-to-end tests of `uninstall` run on Linux and macOS only, because on Windows the script would touch the machine's real PowerShell profiles (the Windows CI test backs them up and restores them).

## 2026-10-29 — `configure` is only hooks and agents; the proxy is set up by `tokred proxy enable`

- **Context:** the proxy is optional, so a command about hooks and agents must not decide things about a server the user may never want (enabling it, its port, whether conversation content is recorded), and `unconfigure` could not mirror such duties.
- **Decision (the separation rule):** `tokred configure` and `tokred unconfigure` touch only hooks and agents, plus what is neutral (the database and the default `tokred.toml`). Everything about the proxy lives in the proxy commands. `configure` writes no `[proxy]` value beyond the defaults of the plain config merge (`enabled = false` stays the code default).
- **`tokred proxy enable [--port <n>] [--record-content <true|false>] [--yes]`** takes the questions over. Both flags take a value (`yes`, `no`, `1`, `0` also work, in any case); `TOKRED_PORT` and `TOKRED_RECORD_CONTENT` are the non-interactive fallback and a flag wins over the variable. In a terminal a value not given is asked: the port is bound on `127.0.0.1` to see that it is free, a port this installation's own proxy serves counts as free, and the default is the saved port, else 1029. Without a terminal, or with `--yes`, only what a flag or variable says is written; the rest keeps the saved value, else port 1029 and content not recorded (privacy first), and a busy port given by flag only warns. It writes `[proxy] bind`, `log_bodies` and `enabled = true` in place (the user's other keys stay) and runs the reconcile. A proxy already running on another port, or with another recording setting, is restarted so the change applies. `--yes` exists because the install script has to be able to say "nobody can answer" even when stdin is a terminal (PowerShell cannot redirect it the way `</dev/null` does).
- **The default of the content question.** `configure` always leaves `log_bodies = false` in the file (it is a default of the merge), so a saved value would always exist and a first `enable` would offer "no". A saved `false` therefore counts as a choice only once the proxy has been enabled (or when it is `true`); before that the default is yes.
- **Install scripts:** after `configure`, `install.sh` and `install.ps1` run `tokred proxy enable`, passing `--port` (script flag or `TOKRED_PORT`). `--no-proxy` (`-NoProxy`, `TOKRED_NO_PROXY=1`) skips the step: hooks and agents only. `--no-configure` skips both, and so does a `configure` that failed. A `configure` whose plan the user declines exits with code 2, like `uninstall`: the scripts read 2 as "stop, change nothing more", skip the proxy step and print no warning, so declining leaves the machine untouched. The installer tests run every install with `--no-proxy` except the dedicated ones, which stop the proxy they start.
- **Consequences:** `tokred configure --yes` on a machine with the proxy on changes nothing about it, so the quiet re-run after an upgrade and `update` never touch the proxy. Someone who runs only `configure` has hooks and agents and no proxy until `tokred proxy enable`; `doctor` shows "disabled" in that state.

## 2026-10-29 — `configure` and `unconfigure` are the switch of hooks and agents; there is no `integration` command

- **Context:** a separate on/off command for hooks and agents would need a remembered "off" state, so that an upgrade does not reinstall what the user turned off. The "configured by version X" marker in the database already is that state: if `unconfigure` left it in place, the first command of a newer binary would re-apply the configuration and silently put back the hooks and agents the user had removed.
- **Decision:** no new command. `tokred unconfigure` is the durable off (it removes hooks and agents, keeps agents you edited, and clears the marker) and `tokred configure` is the on (it installs the version of the binary that runs). The proxy has its own pair, `proxy enable` / `disable`, and the three are independent.
- **Consequences:** nothing to store while off: the binary is the only source of the hook definitions and the agent templates, so an upgrade while off needs no refresh. An empty marker means "not configured" (`needs_reconfigure` ignores it), the same convention as the compression breaker mark.

## 2026-10-29 — No native Windows ARM build

- **Context:** a build for `aarch64-pc-windows-msvc` fails: the `ring` crate's build script runs the C compiler with an option it does not accept (`/imsvc`), and a failed target keeps the whole release from being published.
- **Decision:** the target is left out of `dist-workspace.toml`; `install.ps1` installs the x86_64 build on Windows on ARM, which Windows runs through its emulation. A native build can come back when `ring` (or a pure-Rust provider) builds there.
- **Consequences:** five archives per release; Windows on ARM users run an emulated build.

## 2026-10-29 — A smoke test of the published release, on native runners and Linux containers

- **Context:** the installer tests build a fake release from the just-built binary and serve it from disk, and the Rust tests use the same binary. Neither proves that what was actually published works: the archive and its checksum, the install scripts that ship with the tag, the static musl binary on a machine without glibc, the uninstall scripts of that tag.
- **Decision:** `tests/release/smoke.sh` and `smoke.ps1` install from the real GitHub release of a tag (the release's own install script, nothing set in `TOKRED_RELEASE_BASE_URL`) and assert real state through the whole product (what it checks is listed in [release](release.md)). The workflow `Release smoke` runs it when `Release` succeeds for a `v*` tag, or by hand for a tag. It runs on native runners (Ubuntu, macOS arm64, macOS Intel, Windows) because the installers differ per platform and a hosted runner is the nearest thing to a user's machine, and in `alpine:3.20`, `debian:12-slim` and `fedora:40` containers: Alpine has no glibc, so a green run proves the static musl binary. The scripts come from the branch the workflow runs on, the assets from the tag, so a tag cut before the test existed can be checked. A release is not announced until the workflow is green for its tag.
- **Consequences:** a failing job is a finding about the published artifacts, not about the working tree; the fix is a new release candidate. What stays after an uninstall is asserted and printed as notes, not hidden. The Windows script cannot sandbox the user `PATH` or the PowerShell profiles, so it backs them up and restores them, and the macOS Intel job uses the `macos-15-intel` label because `macos-13` was retired.
- **What it cannot cover:** Claude Code itself with a real login (subscription traffic, real model output, Claude Code's start-up), the branch of `tokred update` that replaces the binary, and a release that is not published yet.
