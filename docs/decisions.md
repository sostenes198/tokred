**English** | [Português](decisions.pt-BR.md)

# Decision log

Format: date, context, decision, consequences. Where the spec diverges from the official Claude Code documentation, the documentation wins and the divergence is recorded here.

## 2026-10-28 — Subagent tool is `Agent`, matcher `Agent|Task`

- **Context:** the spec says to confirm whether the subagent tool is `Task` or `Agent` (section 7.2).
- **Decision:** per the sub-agents documentation the tool is `Agent`, renamed from `Task` in Claude Code v2.1.63, with `Task(...)` still accepted as an alias. The `enforce` hook uses the matcher `Agent|Task` (the matcher syntax accepts `|`-separated exact names).
- **Consequences:** works on both old and new Claude Code versions.

## 2026-10-28 — Hook `timeout` is in seconds

- **Context:** the hooks documentation defines `timeout` in seconds (default 600 for command hooks).
- **Decision:** every tokred hook is registered with `timeout: 5`, as in the spec's example.

## 2026-10-28 — tokred hook commands are recognized structurally, not by the substring `tokred hook `

- **Context:** spec section 5.1 says a tokred entry's `command` "contains the marker `tokred hook `", but its own example is `"/opt/homebrew/bin/tokred" hook guard-read`: the closing quote sits between `tokred` and `hook`, so that substring never occurs in the entries tokred itself writes. A literal implementation would never find (and so never replace or remove) its own entries.
- **Decision:** a command is tokred's when it runs an executable whose file name starts with `tokred` (`tokred`, `tokred.exe`, or a versioned/renamed copy such as `tokred-0.4.0`), optionally quoted, followed by ` hook <name>`. Found by a unit test: under `cargo test` the binary is called `tokred-<hash>`, and an exact-name rule made `configure` add duplicates and `unconfigure` remove nothing.
- **Consequences:** idempotency and unconfigure work for quoted paths, Windows paths with spaces, renamed binaries and bare `tokred hook x`. A wrapper such as `bash -c 'tokred hook x'` is deliberately not recognized.

## 2026-10-28 — Return-format labels are in English

- **Context:** the spec's return format uses `RESULTADO/ARQUIVOS/PENDÊNCIAS`, but section 0 requires agent templates in English.
- **Decision:** the labels are `RESULT`, `FILES` and `PENDING`.

## 2026-10-28 — Documentation file names are lowercase

- **Context:** the spec says `docs/DECISIONS.md` in section 0 and `docs/decisions.md` in section 16.2.
- **Decision:** lowercase, with the `.pt-BR.md` twin. The spec translation (`docs/spec.md`) keeps the original wording.

## 2026-10-28 — Hooks are fail-open stubs in F0

- **Context:** F0 registers the section 7 hooks but their logic arrives in F3 to F5; a user who configures in between must never be broken.
- **Decision:** `tokred hook <event>` drains stdin and prints nothing. `hook` is dispatched **before** argument parsing and always exits 0, even for an unknown event, because Claude Code treats exit code 2 as "block".

## 2026-10-28 — Minimum Rust version is 1.89

- **Context:** edition 2024 needs 1.85, but `rusqlite` 0.41 declares 1.89.
- **Decision:** `rust-version = "1.89"`, verified with `cargo +1.89 check --locked` locally and in CI.

## 2026-10-28 — Data folder from `BaseDirs`, not `ProjectDirs`

- **Context:** the spec lists the data folders per OS and names `directories::ProjectDirs`. On Windows `ProjectDirs` appends `\data` (`%APPDATA%\tokred\data`), which does not match the spec's `%APPDATA%\tokred`.
- **Decision:** use `BaseDirs::data_dir().join("tokred")`, which gives exactly the three paths in the spec.

## 2026-10-28 — `option-ext` (MPL-2.0) allowed by a per-crate exception

- **Context:** `directories` → `dirs-sys` → `option-ext`, licensed MPL-2.0, which is not in the spec's allow-list (MIT, Apache-2.0, BSD, ISC, Unicode, Zlib).
- **Decision:** keep `directories` (the spec's choice) and add a `cargo-deny` exception for that one crate. MPL-2.0 is file-level copyleft and the crate is used unmodified.

## 2026-10-28 — Git remote kept as the existing SSH alias

- **Context:** the spec says `git@github.com:sostenes198/tokred.git`; the repository already had `git@github.com-pessoal:sostenes198/tokred.git`.
- **Decision:** keep the existing remote (the author's choice: use the SSH configuration on their machine).

## 2026-10-28 — Details of `configure` / `unconfigure` not fixed by the spec

- `--claude-dir` is accepted by `configure`, `unconfigure` and `doctor` (the spec lists it only for `configure`); an explicit but invalid path is an error and does not fall back to other candidates.
- `configure` also takes `--force` (the spec mentions it for `update`).
- A blank `settings.json` counts as `{}`; invalid JSON aborts with nothing written, no backup and no data folder created.
- `settings.json` is only rewritten when the merged result differs, and is re-serialized with 2-space indentation (key order is preserved, original whitespace is not). The backup holds the exact previous text.
- Round-trip limit: an event array or `hooks` object that was **already empty** and that tokred merged into is removed by `unconfigure`, because "emptied by our removal" cannot be told apart from "empty to begin with". Empty containers tokred did not touch are preserved.
- If `settings.json` did not exist, `unconfigure` leaves `{}` behind.
- `unconfigure` keeps agent files you edited (and, when the database is gone, every agent file, since their origin cannot be verified).
- `--purge` refuses to delete a folder that has neither `tokred.db` nor `tokred.toml`.
- The database is created only after the plan is confirmed.
- `tokred.toml` is rewritten (without comments) only when keys were missing; otherwise it is left byte for byte.
- `unconfigure` is lenient: it removes tokred's handlers and leaves anything it does not understand (a non-object `hooks`, an event that is not an array) exactly as it is. Only `configure`, which has to write into those places, reports a bad shape, and only for the events it writes to.
- Terminal I/O goes through a small `Io` seam (interactive flag, input, output), and command output is rendered by pure functions into `String`s. This is what lets the prompts, the plan text and every `doctor` state be unit-tested without a TTY, and what keeps line coverage at 100%.
- `doctor --bench`, price sync and `tokred metrics` are not part of F0 and are omitted from the quick start until their phases.

## 2026-10-28 — F6 (artifact summaries) and F7 (response cache / proxy) are deferred until there is data

- **Context:** the spec gates both phases on measurements: F6 "only after measuring F1 to F5" and F7 "decision recorded in DECISIONS.md based on the data from the metrics". The tiered cache itself (L1/L2/L3, `cache stats`, `cache prune`, per-root file counts) is part of F1 to F5 and is implemented. What would be new in F6 is storing *summaries of artifacts keyed by content hash* and serving them instead of the artifact, which needs a way to produce the summaries (tokred cannot call a model; it would have to capture the output of the `tokred-summarizer` subagent) and a hook to serve them. No usage data exists yet to say that is worth its complexity.
- **Decision:** do not build F6 or F7 yet. Collect data with the shipped tooling and decide with it.
- **F6 go criteria** (check with `tokred stats` and `tokred metrics` after about four weeks of real use): `large_file` denials plus `reread` denials are a meaningful share of guard events (suggested threshold: more than 20% of guarded reads) **and** the estimated tokens avoided by the guards are small compared with the real input cost (suggested: under 5%), which would mean the guards are blocking without replacing the content; and the L3 hit rate of the cache shows repeated access to the same artifacts.
- **F7 go criteria:** a measured share of byte-identical repeated prompts above 10% (none is expected: agentic prompts rarely repeat, hooks cannot intercept API calls, and Anthropic already caches the prompt prefix server-side). Without that evidence the proxy stays out of scope, as the spec's non-goals say.
- **Consequences:** nothing is lost: the data model and the hook events already record what the criteria need. Revisit this entry when the criteria can be evaluated.


## 2026-10-28 — Hooks: where the official documentation overrides the spec

Confirmed against <https://code.claude.com/docs/en/hooks> while implementing phases F3 to F5:

- `warn` mode emits `additionalContext` only. It never emits `permissionDecision: "allow"`, because that would skip the user's own permission prompt.
- `Stop` emits `systemMessage` only (a message for the user). `additionalContext` or `decision` on `Stop` would make Claude continue, which the spec forbids. `SessionStart` prints nothing, because its stdout becomes model context.
- `SessionStart` `source` has a fifth value, `fork`; like `startup` and `resume` it is not a cut (only `clear` and `compact` are).
- File reads are tracked per `session/agent_id`, because a subagent's context is separate from its parent's. A read in the same second as a cut is allowed (strict `>`).
- Deny globs match the path relative to the session's `cwd`. Images and PDFs are exempt from the size check. A `Read` with any argument besides `file_path`, `offset` and `limit` (such as `pages`) skips the re-read check. Bytes avoided by a ranged read are estimated at 80 bytes per line, and files above 16 MiB are not hashed.
- `guard-search` counts files per repository root (cached in the tiered cache, keyed by root, threshold and root mtime), stops counting at threshold + 1 and gives up after 100 ms, in which case it allows the search.
- `route` stores only actionable classes in `classifications` (never `other`), caches classifications by `blake3(prompt)` plus a rules version, and behaves like `warn` in `enforce` mode. For `enforce`, architect and planner are interchangeable.
- `session-check` warns once per session, again after the context grows by half the limit, or after a clear/compact. A literal `~/` in `transcript_path` is not expanded.
- stdout cannot be closed (`unsafe` is forbidden): the answer is flushed and the database row is written last, but Claude Code sees the end only when the process exits. A hook never creates the data folder or `tokred.db`; an invalid `tokred.toml` means defaults. A watchdog (`TOKRED_HOOK_GUARD_MS`, default 2 s) turns a stuck hook into a `timeout_guard` outcome.
- Measured on a quiet development Mac with a release build: spawn-to-exit p95 of 10 to 12 ms (about 7 ms of that is the spawn baseline), internal p95 up to about 6 ms. Under load p95 reached 37 ms. All within the 50 ms budget.

## 2026-10-28 — Pricing, update and the self-update path

- The price snapshot keeps LiteLLM's own shape with normalized keys, so one parser reads both the live list and the embedded file (39 rows, regenerated by `scripts/update-price-snapshot.sh`). Key preference: bare Anthropic keys, then `anthropic/`, `vertex_ai/`, `bedrock/`, then Bedrock-native `anthropic.`; regional and reseller keys are dropped. Missing cache prices are derived (write 1.25x, read 0.1x of input).
- "Latest price of a family" is the entry with the most recent date in its model id; ids without a date rank below dated ones.
- A sync that returns no rows never replaces the table. `price_sync_attempt_at` is recorded so a failed sync is not retried on every command.
- `BinarySwap` is a port owned by `tokred-update` (the `FileSystem` port has no rename or chmod). Self-update verifies the SHA-256 sidecar before touching anything, replaces the binary atomically, and on Windows renames the old `.exe` to `.old` and removes it on the next run. Linux always maps to the musl build. Release tags are `v<version>`.
- After a self-update the configuration is re-applied by running the **new** binary (`configure --yes`), so the new agent templates are the ones installed. For package-manager installs, the first CLI command after an upgrade re-applies the configuration quietly (`needs_reconfigure`).
- `TOKRED_OFFLINE`, `TOKRED_PRICES_URL` and `TOKRED_RELEASES_URL` are additions to the spec: one switch for every network call, and URL overrides for mirrors and for tests against a local server.

## 2026-10-28 — Metrics calculations

- The "everything on opus" counterfactual uses the **latest** opus price in the table. When the model actually used is a more expensive, older opus, the counterfactual can be lower than the real cost; the routing saving is then negative, which is accurate.
- A guard "block" is a `guard-*` event with decision `deny`; `warn` saves nothing. Guard savings are always labelled an estimate. The session's model for pricing them is the one with the most messages (ties: smaller id; fallback opus).
- The projection divisor is `min(window, days since the first row)`, at least 1. `TimeRange::last` is `[now - span, now + 1)`.
- A file replaced by a larger one is not detected by ingestion because there is no content hash; a shrunk file restarts from zero.
- `route` classifications are counted from `route` hook events, so there is no separate classification table to query for the report.

## 2026-10-28 — Audit: locations confirmed in the official documentation

- MCP servers are not configured in `settings.json`. User scope is the top-level `mcpServers` of `~/.claude.json`, local scope is `projects["<path>"].mcpServers` in the same file, project scope is `<project>/.mcp.json`; precedence is local, then project, then user. `settings.json` only contributes `disabledMcpjsonServers`. Whether `.claude.json` moves with `CLAUDE_CONFIG_DIR` is not documented, so `<claude>/.claude.json` is tried first, then `~/.claude.json`.
- Project memory also includes `.claude/CLAUDE.md`, `CLAUDE.local.md`, ancestor directories and always-on `.claude/rules` files. Imports follow the documented 4-hop limit and skip fences, code spans and block HTML comments.
- The spec's "split with @import" suggestion is replaced by "move to path-scoped rules or a skill", because imports do not reduce context cost. Skill bodies load only on invocation, so skills also report their start-up weight. Thresholds: 200 lines for `CLAUDE.md` and 500 lines for skills (both from the docs), and 2000 tokens for agents (a tokred heuristic).
- Plugin skills, agents and servers, `commands/*.md`, claude.ai connectors and managed servers are not enumerated; usage seen in transcripts for servers not configured locally is listed separately.

## 2026-10-28 — Release pipeline and licenses

- `dist` builds six targets (macOS, Linux musl and Windows on x86_64 and aarch64) with `.tar.gz` / `.zip` archives and `.sha256` sidecars. The agent templates moved from `templates/agents/` into `crates/tokred-install/templates/` because `include_str!` from the repository root would break the published crate.
- `dist` cannot add Homebrew caveats, so `publish-package-managers.yml` waits for the formula of the new version to land in the tap and patches the caveat into it. The Scoop manifest, the winget pull request and `cargo publish --workspace` are also in that workflow. None of it has run yet: it needs the four repository secrets and a tag, and the winget package needs its first manifest submitted by hand.
- `deny.toml` allows `CDLA-Permissive-2.0` (the Mozilla CA certificate data bundled by `webpki-roots`, pulled in by the HTTP client) in addition to the `MPL-2.0` exception for `option-ext`.

## 2026-10-28 — Terminal dashboards, background chores and the benchmark

- `tokred stats`, `stats latency` and `metrics` print a colored dashboard in the terminal (the HTML report is the same data, not a replacement). Color follows `NO_COLOR` (always wins), `CLICOLOR_FORCE`, a dumb `TERM`, and whether stdout is a terminal.
- Notices from background chores go to **stderr** so `--json` output stays clean. Chores run only after commands that are not hooks, are best effort, and never fail the command: the daily price sync, the daily version check and the quiet re-application of the configuration after an upgrade.
- `doctor --bench` runs each hook as a separate process in a scratch data folder with its own database (so the hooks do real work and the user's data is untouched), times it from spawn to exit, and stores the difference from the hooks' own measurements as the process start-up overhead in `meta`. `TOKRED_BENCH_RUNS` overrides the default of 50 runs per payload (used by the tests).
- Line coverage is 100% across the workspace. LLVM's summary can read a fraction of a percent lower because it counts macro regions and `?` error branches separately; the per-line views (`lcov`, annotated text) are the reference.


## 2026-10-28 — Spec v3: the proxy, and a model policy that respects the user's choice

- **Context:** the v2 draft proposed a local proxy with tool-result compression, an exact-match response cache and measurement, but no model routing. Reading the official gateway guide, the prompt-caching page and the preserved-thinking page showed that (a) Claude Code labels every request (`x-claude-code-request-class`, agent type, agent id, prompt id, session id), (b) gateways are told to inspect without modifying and a rewritten history can trigger a "bound to a different conversation" error, (c) changing the main model mid-session rebuilds the whole prompt cache, and (d) auxiliary requests may run on the main model and some reuse the conversation's cache.
- **Decision (agreed with the author):** the proxy classifies requests and may choose the model **only** for subagents and auxiliary requests; the **main loop keeps the user's configured model** and tokred only **summarizes and suggests** a cheaper one when the task was simple, preferably when the cache is already cold. Compression and the response cache are **shadow-mode only** until a data gate passes. Every behaviour that alters traffic starts in `observe` mode.
- **Consequences:** `docs/spec.md` (v3) replaces v1 and the v2 draft, which are kept in `docs/archive/`. The old F6/F7/F9 phases are superseded by F6 (stabilization), F7 (spike), F8 (pass-through and measurement), F9 (model policy), F10 (suggestions and shadow mode) and F11 (optional active compression). The Windows CI failure, the missing model families (Fable, Mythos) and the per-model cache multipliers are tracked as known problems to fix first.


## 2026-10-28 — F7 spike, first results (API-key login; subscription not yet tested)

Run with a throwaway pass-through proxy (`scripts/spike/`) and the real Claude Code 2.1.312 in print mode, using this machine's API-key login (an organization-managed key, `/login managed key`).

- **Pass-through works.** Streaming (SSE) is forwarded unchanged and `usage` is readable from `message_start` / `message_delta`. The response `model` matches the request.
- **Headers actually sent:** `x-claude-code-session-id` on every request and `x-claude-code-agent-id` on subagent requests only. `x-claude-code-request-class`, `x-claude-code-agent-type` and `x-claude-code-prompt-id`, which the gateway guide says are sent, did **not** appear in this version and mode. Consequence: classification cannot depend on them. Main versus subagent is told apart by the presence of `agent-id`; the kind of subagent by its tool set (the Explore subagent had 10 read-only tools against 15 for the main loop) and system blocks. The documented headers are a bonus when present, to be re-checked in interactive mode.
- **Subagents build their own cache** (first turn wrote about 19.9k tokens, the next turns read them back), which is why a subagent's model must be decided once and kept.
- **Rewriting the model of subagent requests works.** With the main loop on Sonnet, rewriting only the subagent's requests to Haiku was accepted by the API on all three turns (including tool results being sent back), Claude Code showed no problem with a response from a different model, and the final answer was correct. The main loop's requests were never touched.
- **Not yet verified:** a claude.ai **subscription** login (this machine has none: `authMethod` is `api_key`); interactive mode (the extra headers may appear there); `auxiliary` requests (none occurred in print mode); compaction; thinking-heavy models across a rewrite. The probe and exact steps to test a subscription without touching a real login are in `scripts/spike/README.md`.
- **Decision:** the main loop's model is **never** changed, with no configuration option to do so; tokred only advises.


## 2026-10-28 — F7 spike, subscription result (claude.ai login): it works

Same throwaway probe and Claude Code 2.1.312 as the API-key run, now on a claude.ai **subscription** login (`authMethod: claude.ai`, `ANTHROPIC_API_KEY` unset), through `ANTHROPIC_BASE_URL` only.

- **Pass-through works on a subscription.** Status 200 with streaming. The credential travels in the `authorization` header (OAuth), not `x-api-key`, and the `oauth-2025-04-20` value of `anthropic-beta` is present, so a proxy must forward `authorization` and the whole `anthropic-beta` header untouched, exactly as the gateway guide says. The subscription request also carried `extended-cache-ttl-2025-04-11` (the 1-hour cache).
- **Headers are the same as with an API key:** `x-claude-code-session-id` on every request and `x-claude-code-agent-id` on subagent requests only; no `request-class`, `agent-type` or `prompt-id` in `-p` mode.
- **Rewriting a subagent's model works on a subscription.** With the main loop on Sonnet, rewriting only the subagent's three requests to Haiku was accepted (status 200 each time) and the final answer was correct. The main loop's two requests were never touched.
- **Not covered, so still open:** Anthropic's usage terms for subscription traffic through a proxy (not verifiable by testing; it must be read and recorded before the proxy is recommended for subscriptions); per-model plan limits (this test only moved Sonnet to Haiku, a cheaper model); interactive mode, `auxiliary` requests and compaction; very long sessions.
- **Consequence:** the F7 go/no-go is **go** for both login types from a technical point of view. The proxy stays opt-in, and the subscription recommendation depends on the terms-of-use check.


## 2026-10-28 — F7 spike, interactive mode: how to recognize auxiliary requests

Interactive `claude` (2.1.312, Opus 5.5, API-key login) through the probe, two short prompts.

- **`x-claude-code-request-class` is not sent in interactive mode either.** Only `x-claude-code-session-id` arrived (and `x-claude-code-agent-id` for subagents in the earlier runs). The documented request-class, agent-type and prompt-id headers cannot be relied on in this version.
- **The auxiliary request is recognizable by its shape.** Besides the main-loop turns (16 tools, adaptive thinking), one request had **no tools, no thinking and exactly one message** (1,167 input and 21 output tokens, most likely the session title). It ran on the main model (Opus 5.5) and read and wrote **no** prompt cache, so moving it to a cheaper model loses nothing.
- **Startup checks:** twelve requests with `max_tokens: 1`, no tools and no system prompt returned `429` and were retried by Claude Code; the proxy must (and did) pass `429` through unchanged.
- **Decision:** a request is treated as auxiliary only when it has no `agent-id`, no tools, no `thinking` and exactly one message (or when a future Claude Code sends `x-claude-code-request-class: auxiliary`). The main loop always carries tools, so it can never match. Everything else without an `agent-id` is the main loop and is never rewritten.


## 2026-10-28 — F6 stabilization

- **1-hour cache writes.** Current transcripts split `cache_creation_input_tokens` into `cache_creation.ephemeral_5m_input_tokens` and `ephemeral_1h_input_tokens`, and on this machine every write was 1-hour. LiteLLM publishes the price as `cache_creation_input_token_cost_above_1hr`. `Usage` gained `cache_write_1h` (a part of `cache_write`) and `ModelPrice` gained `cache_write_1h`; when the price list lacks it the documented rate of 2x input is used. Migration v3 adds the columns and clears the ingestion offsets so existing rows are backfilled without duplicates.
- **Family ranking.** Versions now rank before dates (`claude-opus-5-5` > `claude-opus-4-7-20260416`). The previous rule, which ranked undated ids below dated ones, made the counterfactual price everything at Opus 4.7.
- **Fable and Mythos** are families. The `Family` type stays a closed enum (unknown families still resolve to "unpriced") rather than the open type the spec suggested: it keeps exhaustive matching, and an unknown family is already handled.
- **Windows.** The walk's cached metadata was replaced by a fresh `std::fs::metadata` read, and CI runs `cargo test --no-fail-fast` so every failing test binary shows up, not only the first.
- **Not done:** exercising the release pipeline with a test tag. Pushing a tag is outward-facing (it can publish to Homebrew, Scoop, winget and crates.io once the secrets exist) and needs the author's explicit go-ahead.


## 2026-10-28 — Subscription traffic through the proxy: what the terms say

Checked against Anthropic's Consumer Terms (Section 3, item 7) and the Claude Code gateway documentation. This is a reading of the published text, not legal advice.

- The Consumer Terms forbid accessing the Services "through automated or non-human means" except with an API key "or where we otherwise explicitly permit it". They do not mention local proxies or gateways by name.
- The Claude Code documentation explicitly describes the configuration tokred uses: with only `ANTHROPIC_BASE_URL` set, "a saved claude.ai login remains the active credential" and gateways "must forward the OAuth capability in `anthropic-beta`". **Measuring** subscription traffic through the local proxy is therefore a documented setup.
- **Rewriting the model** of subscription requests is not addressed by either text. It lowers usage rather than bypassing limits, but it is not explicitly permitted. Decision: routing stays in `observe` by default for every login, and `doctor` says when `enforce` is on with a subscription login, so turning it on is an informed choice of the user.


## 2026-10-28 — The proxy asks the upstream for `Accept-Encoding: identity`

- **Context:** the real API gzips its responses when the client allows it, and the proxy has to read `usage` from the response body to measure tokens exactly.
- **Decision:** the proxy rewrites `Accept-Encoding` to `identity` on the request it sends upstream. It is the only request header it rewrites; every other header is forwarded as it came.
- **Consequences:** responses arrive uncompressed (a few more bytes between the proxy and the API), so streaming and `usage` can be read as they pass. If an upstream compresses anyway, the response is forwarded unchanged and its usage is not read (fail-open, see the known risks below).

## 2026-10-28 — Wiring of the background proxy

- **Context:** `tokred proxy start` has to leave a proxy running after the command returns, and `proxy stop` and `tokred claude` have to find it and talk to it safely.
- **Decision:** the background proxy is the same binary, started as `tokred proxy start --foreground` (the user service runs the same command), so there is no second executable to install or keep in sync. State lives in `<data>/proxy/`: `secret` (owner-only, `0600` on Unix), which the admin endpoints (`/__tokred/...`) require, and `pid`. `proxy stop` sends the shutdown request and **waits until the process is gone** (it fails if the proxy did not stop) before clearing the pid file.
- **Consequences:** `proxy stop && proxy start` is safe to script (`stop` returns only after the proxy stopped answering); a stale pid file is ignored when the proxy does not answer its health endpoint.

## 2026-10-28 — The compression circuit breaker survives a restart

- **Context:** when rewritten requests keep being rejected, the breaker turns compression off. In memory only, a restart would turn it on again and the same failures would repeat.
- **Decision:** the trip is persisted in the database `meta` table (`compression_tripped_at`). While it is set, the proxy starts with compression in `observe` even if `tokred.toml` says `enforce`; `tokred compression on` clears it.
- **Consequences:** a restart (or a reboot with the user service) never silently re-enables a feature that was failing; `tokred compression status` shows that the breaker tripped.

## 2026-10-28 — `tokred compression on|off` edits Claude Code's user config only when run

- **Context:** reversible compression needs the `tokred_retrieve` MCP server registered in Claude Code, which means editing its user config (`~/.claude.json`), a file that holds more than tokred's data.
- **Decision:** only `tokred compression on` (registers the server and sets `[compression] mode = "enforce"`) and `off` (unregisters it and sets `observe`) touch that file, and only when you run them. The previous content is saved next to it as `.tokred.bak` first. `configure`, `tokred claude` and the proxy never edit it.
- **Consequences:** installing or using the proxy changes nothing in Claude Code's configuration; turning compression on is an explicit, reversible step.

## 2026-10-28 — Known risks of the proxy

- **Compaction looks like a simple prompt.** Compaction requests carry no distinguishing header in the Claude Code version we tested, so one could be misread as a simple prompt and feed a cheaper-model suggestion. Suggestions only advise (the main loop's model is never changed), and `routing.compaction` stays `keep` by default, so the cost of a wrong reading is a wrong suggestion.
- **Usage not read when the upstream compresses anyway.** If a response arrives compressed despite `Accept-Encoding: identity`, it is forwarded unchanged and that request's usage is simply not recorded (fail-open: Claude Code is never affected, the measurement has a gap).
- **Routing stays `observe` by default** on every login, subscription logins included, because Anthropic's terms do not address rewriting the model of subscription requests (see the decision on subscription traffic above). Turning `enforce` on is the user's informed choice, and `doctor` says when it is on with a subscription login.

## 2026-10-28 — Windows: a captured `proxy start` waits for the proxy

- **Context:** the first CI run of the proxy end-to-end tests hung on Windows only. The test harness captures the output of `tokred proxy start` through pipes; the detached proxy inherits those pipe handles (Windows passes every inheritable handle of the parent to a child, whatever the child's own standard handles are), so the harness never sees the end of the output until the proxy exits.
- **Decision:** the tests start the proxy without capturing its output, and the limit is documented in [proxy](proxy.md). Closing the inherited handles needs `unsafe` or a platform crate, which the workspace does not allow today; an alternative to evaluate later is starting the process through the shell (`Start-Process`), which does not inherit handles. CI jobs now stop after 20 minutes so a hang fails with its log.
- **Consequences:** a terminal, a file redirect, `NUL` or the user service all work; capturing the output of `proxy start` in a pipe on Windows blocks until the proxy stops.

## 2026-10-29 — Corporate networks: HTTP proxy tunnel and extra certificate authorities

- **Context:** the proxy connected straight to the API with the bundled Mozilla roots, so it failed behind a corporate HTTP proxy and behind one that inspects TLS with its own authority.
- **Decision:** the upstream client tunnels with `CONNECT` through the proxy named by `HTTPS_PROXY`, else `ALL_PROXY`, else `HTTP_PROXY` (and the lower-case names; `HTTP_PROXY` is a fallback because many corporate images only set that one), honouring `NO_PROXY` with the curl rules. This uses `hyper-util`'s `Tunnel` and proxy matcher (the `client-proxy` feature; no hand-written handshake). Only `http://` proxies are supported and only for an `https` upstream (a loopback `http` upstream is always direct). Trust is the bundled roots plus the operating system's store (`rustls-native-certs`, `ring` only) plus the PEM files of `[proxy] ca_bundle` (new optional key), `NODE_EXTRA_CA_CERTS` and `SSL_CERT_FILE`, parsed with `rustls-pki-types` (not the unmaintained `rustls-pemfile`). Unsupported (`socks5://`, `https://`) or malformed proxy values, and missing or empty CA files, are logged once by variable or file name (never the value, which can hold a password) and ignored: the proxy connects directly or with the remaining roots, so tokred never breaks Claude Code. A reachable proxy that refuses the tunnel, or an untrusted certificate, is the usual `502` for an unreachable upstream. The settings are read once at start, from the environment of the process that starts the proxy; `doctor` and `proxy status` show the proxy host (no credentials) and the CA files.
- **Consequences:** TLS-inspecting and tunnelling corporate networks work with the variables Claude Code already uses. New dependencies: `rustls`, `rustls-native-certs`, `rustls-pki-types`, `webpki-roots` (already in the tree through `hyper-rustls` and `ureq`) and, for tests only, `rcgen` and `tokio-rustls`; `cargo deny` passes and there is no `aws-lc`. SOCKS and TLS-to-the-proxy stay out of scope. Environment changes need a proxy restart.

## 2026-10-29 — Windows: the background proxy starts through `Start-Process`

- **Context:** follow-up to "Windows: a captured `proxy start` waits for the proxy" (2026-10-28): the detached proxy inherited the pipes of a capturing caller.
- **Decision:** on Windows `spawn_detached` runs `powershell.exe -NoProfile -NonInteractive -Command "Start-Process -FilePath <exe> -ArgumentList <args> -WindowStyle Hidden -PassThru | Select-Object -ExpandProperty Id"`, which starts the proxy without this process's handles, and reads the pid from the first line of its output (only the first line, so a stray open pipe cannot hold the call up). Paths and arguments are quoted by pure functions tested on every operating system; a plain detached spawn (the old behaviour) is the fallback when PowerShell cannot be used. No `unsafe` and no new dependency. The end-to-end test that runs `proxy start` with captured output runs on every operating system (it also guards Linux and macOS). It waits for the command on a thread with a 60-second deadline instead of relying on the command's own timeout, because killing the command does not help while the proxy still holds the captured pipes: the harness would wait for them forever.
- **Consequences:** `out=$(tokred proxy start)` should return on Windows too and the limit is removed from [proxy](proxy.md). This could not be run where it was written; CI on Windows is the first real check, and the fallback keeps the previous behaviour if PowerShell misbehaves.

## 2026-10-29 — The proxy's requests and the transcripts are merged once, before any metric

- **Context:** with the proxy running every request is in `proxy_requests` and in the transcripts (`usage`); adding the two counts each twice, and using only one loses either the exact data (model that answered, 1-hour cache split, latency) or the requests made while the proxy was off (spec 6.4 and 7.2).
- **Decision:** `merge_usage` (pure, in `tokred-metrics`) runs once, where the data is loaded, so `stats`, the terminal dashboards, the HTML report, the projection and the before/after read one list. A proxy request is paired with the transcript row of the same `message_key`; only a proxy request **without** a key falls back to the same session, equal tokens (the four counters, not the 1-hour split) and a time within 5 seconds of the request or up to 5 seconds after the end of its response (the transcript line is written when the response ends, so a plain "within 5 s of the arrival" would miss any answer longer than 5 s). Pairing is one to one and nearest in time first, so two identical requests in the same seconds are never collapsed. A pair takes the proxy's numbers and the transcript's project and key; unpaired rows of both kinds stay. A request answered with a non-2xx status or without tokens is not a message. Two proxy rows with the same id, or the same message key, count once. `usage` rows gain the optional `origin` and `agent_type` (never stored) so `--by origin` and `--by agent-type` need no second data path; without the proxy `origin` falls back to `main`/`subagent` from the transcripts.
- **Consequences:** a proxied request is counted once whatever the order of ingestion; the routing saving prices the model that really answered; with no proxy rows the transcript rows pass through untouched, so every output is what it was. A key that is present on both sides but different is two requests (never merged by time). Covered by unit tests, a snapshot and properties (nothing lost, nothing repeated, merging again changes nothing).

## 2026-10-29 — `stats` shows the pending suggestion and never marks it shown

- **Context:** the suggestion's `shown` flag was documented as set by the status line, the message or `stats`, but the status line deliberately never marks it (it is redrawn constantly) so it does not hide the suggestion from the one-line message; `stats --json` must also not write to the database.
- **Decision:** `stats` shows the latest suggestion of the period that is not shown yet, with a pointer to `tokred suggest`, and does not change the flag. Docs ([proxy](proxy.md), [routing](routing.md)) list `tokred stats` among the places suggestions appear.
- **Consequences:** `stats` is read-only and repeatable; the same suggestion stays in `stats` until the message channel shows it. The doc comment on `shown` now says it is set by the message channel only.

## 2026-10-29 — The proxy sections of the HTML report appear only when the proxy recorded something

- **Context:** the report's rule is that a section without data shows a "no data yet" notice; the task for this change is to keep every existing output (and snapshot) identical when the proxy tables are empty.
- **Decision:** the new sections (model routing, overhead, cache misses by cause, cost by origin and agent type) are rendered only when the report has proxy data, and the data is embedded in the page only then (`proxy` is skipped when empty). Inside them an empty part (no cache misses, no decisions) is left out rather than shown empty.
- **Consequences:** a report without the proxy is byte for byte what it was; the "no data yet" rule holds for the original eight sections.

## 2026-10-29 — One function builds the report from the repositories

- **Context:** the proxy will serve the same report at `/metrics`, and `tokred-proxy` may not depend on the store or the report crates.
- **Decision:** `report_html(repos, prices, now, since, audit)` in `tokred-cli` takes the repository ports (usage, proxy requests, hook events, routing, suggestions, meta) and returns the HTML as a string: no transcript ingestion, no file or network access, no price sync. `tokred metrics` ingests the transcripts, then writes what `report_html` returns. A later change can expose it to the proxy through a trait in `tokred-core` implemented in `tokred-cli`.
- **Consequences:** the file and the page cannot drift apart. The route itself is not part of this change.

## 2026-10-29 — Richer proxy request records and migration numbering

- **Context:** `proxy_requests` lacked the stop reason, the tool names and the cache lifetime, and did not say which routing mode a request ran under.
- **Decision:** four columns are added by migration **v6** (`stop_reason`, `tools` as JSON text with names only and at most 200, `cache_ttl` as `5m`/`1h`/null, `routing_mode`); old rows keep null/empty values. `stop_reason` and the tool names were already parsed (response tap and request shape), so nothing new is parsed on the request path. `cache_ttl` is computed after the response from the body that was sent, looking only at the places Claude Code puts breakpoints (top level, `system`, `tools`, message content blocks); a `cache_control` without `ttl` counts as `5m`, and the longest one wins. `routing_mode` comes from the `[routing]` mode the proxy was assembled with, not from the hook notes: the notes are reset when a hook fails, which are exactly the requests worth explaining. Version 5 is reserved for the dashboard workstream's `captured_turns` table, so this branch carries a no-op v5 placeholder to keep v6 at its final number; the merge replaces the placeholder with that migration.
- **Consequences:** `Store::latest_schema_version()` is 6 on this branch. `tokred-store` still needs no JSON library: the tool names are serialized in `tokred-core`.

## 2026-10-29 — `retention_days` prunes every proxy table, on a schedule

- **Context:** nothing ever deleted proxy data, although `[proxy] retention_days` existed.
- **Decision:** when the proxy starts, and then every hour, rows older than `retention_days` are deleted from `proxy_requests`, `routing_decisions`, `prompt_summaries`, `suggestions`, `shadow_savings`, `stored_results` and `compressed_blocks`, on a thread of its own (the database work never runs on the request path). `0` keeps everything, and that rule lives in the pruning function because the older `prune_proxy_requests(now, 0)` means "delete everything before now". The kept tool results and the replacements of a conversation that still has a request inside the period are kept whatever their age: a replaced tool result must be sent as the same bytes every turn (changing it breaks the prompt cache) and the model may still ask for the original. Records of a request in flight are never at risk: they are written when the response ends, with the time the request arrived, which is within the last day at worst.
- **Consequences:** a long-lived conversation keeps its compression data longer than `retention_days`; the cost is bounded by how many conversations are active.

## 2026-10-29 — `[proxy] enabled` is the single switch; `tokred initialize` reconciles Claude Code's settings

- **Context:** the spec had an `enabled` flag that nothing read and an optional `persist_base_url`, with the dangerous consequence that with the variable persisted and the proxy down Claude Code cannot reach the API. The installer now puts a `claude` shell wrapper in place that can run a command before every launch.
- **Decision:** `enabled` is the only user-facing switch. `false`: `tokred claude` runs `claude` directly (one line on stderr), `tokred initialize` removes tokred's URL from Claude Code's `settings.json`, `proxy start` and `proxy service install` refuse, `doctor` shows "disabled"; `proxy stop` and `status` still work and a running proxy is never stopped. `true`: `initialize` starts the proxy when it is not running and, once it answers, sets `env.ANTHROPIC_BASE_URL`; if the proxy cannot be started the URL is removed again (it is only written after the proxy answers, so there is no window with a dead URL) and a warning is printed. `configure` sets it to `true` only when the user had no value (an explicit value is kept; the check runs before the defaults are merged in, which would otherwise always add `enabled = false`), and `unconfigure` leaves `tokred.toml` alone but removes tokred's URL from `settings.json`. `tokred proxy enable|disable` edit `tokred.toml` and run the same reconcile at once.
- **`persist_base_url` is removed.** With `enabled` keeping the URL in `settings.json`, launches from an IDE that never run the wrapper already use it, so the key added nothing but a second way to get the same state. Files that still have the key keep working (it is ignored). The risk it carried is covered by three safeguards that now hang on `enabled`: `initialize` never leaves a URL pointing at a proxy it could not start; the `session-start` hook starts a dead proxy; `doctor` warns. This supersedes the `persist_base_url` item of the spec (section 7), including the "restore the previous value" part: another gateway is never overwritten, so there is never a value to restore.
- **A value that is not tokred's is never touched.** "Tokred's URL" is the current proxy URL or a URL tokred wrote before, recorded one per line in `<data>/proxy/base_urls` (a plain file, so the per-launch path does not open the database, and so a changed port is followed). The value exported in the shell counts as a current value too. Another gateway, even on `127.0.0.1`, makes `initialize` print one warning and do nothing (and start nothing). The file edit goes through `tokred-install` (pure merge functions with fixtures and property tests): it changes only that key, keeps its position and the other `env` keys, writes nothing when nothing changes and makes no backup (the backup `configure` made must keep the user's original).
- **`tokred initialize` is fail-open and quiet.** It always exits 0 (invalid arguments aside; even an unusable environment is a warning), prints nothing unless there is a warning (`--verbose`, `--json` say more), reads one file and probes the proxy once, and does not open the database. It settles on the Claude Code folder `claude` itself would use: `$CLAUDE_CONFIG_DIR`, else the folder saved by `configure`, else `~/.claude`.
- **The `session-start` safety net** is composed in the binary, after the hook has answered, and not in `tokred-hooks` (which has no network code): when `enabled` is true, the session's `ANTHROPIC_BASE_URL` (environment, else `settings.json`) is tokred's and the proxy does not answer a 200 ms probe, it starts the proxy detached and returns without waiting. It is gated on `enabled`, so the hook benchmark and the hook tests never start a proxy.
- **Upgrades and the user service.** The old `configure` wrote every default, so existing installations have an explicit `enabled = false`; it is kept (the user's value wins) and the features stay off until `tokred proxy enable` is run once, which the changelog and the configuration page say. While disabled, `proxy start --foreground` refuses, so an installed user service would fail and be restarted by the service manager: `proxy service install` refuses while disabled and `proxy disable` says to run `proxy service uninstall` when a service is installed.
- **Windows.** The `session-start` safety net and `initialize` start the proxy detached with no standard streams, but on Windows a child still inherits inheritable handles such as the pipe Claude Code reads a hook's output from, so the caller may wait for its own timeout (the limit already documented for `proxy start`). The spawn is kept: waiting is better than Claude Code being locked out by a dead proxy, and the hook still exits 0. Not verified on Windows.
- **Consequences:** a fresh install has the proxy on, started by the first launch through the wrapper; `tokred proxy disable` is the way out of any trouble, and so is deleting the key by hand.
