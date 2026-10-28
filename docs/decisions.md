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
