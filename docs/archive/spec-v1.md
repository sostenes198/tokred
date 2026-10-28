> **Archived.** Superseded by the current specification: [spec.md](../spec.md).

**English** | [Português](spec-v1.pt-BR.md)

# tokred — Implementation specification

**Repository:** https://github.com/sostenes198/tokred

**Short description (GitHub "About"):** *Token and cost reducer for Claude Code — model routing, context guards and local metrics. Works alongside rtk.*

**Description:** tokred is a local, cross-platform Rust CLI that reduces Claude Code's cost and token consumption. It installs subagents with the right model for each kind of task (opus for architecture and planning, sonnet for development, haiku for search and summarization), hooks that enforce that delegation and block useless context reads, and a metrics dashboard with tokens, cost, savings and added latency. Everything runs on the user's machine, with its own SQLite database and no external service. It was designed to run alongside rtk (rust token killer), which compresses command output; tokred takes care of the rest.

## 0. Instructions for Claude Code

- Before implementing **each hook**, consult the official Claude Code hooks documentation (https://code.claude.com/docs/en/hooks). Check the input format (stdin), the output format (`hookSpecificOutput`, `permissionDecision`, `additionalContext`, `systemMessage`) and the current name of the subagent tool (`Task` or `Agent`). **Do not assume memory field names.** Where this spec diverges from the docs, the docs win: record the divergence in `docs/DECISIONS.md`.
- Implement in phases (section 12). At the end of each phase: tests passing, `cargo clippy -- -D warnings` clean, and a summary of what was done.
- Ask before any decision that changes the behavior described here.

### Language rules

- **Code in English:** names of folders, files, crates, modules, types, functions, variables, database tables and columns, config keys, comments, commit messages and every string displayed by the CLI and the hooks (including `additionalContext`, warnings and the metrics HTML).
- **Subagent templates in English.** The classifier keywords remain in pt-BR and en, because they are data used to recognize prompts in both languages.
- **Documentation always in both languages:** every document has an English version and a Portuguese version, with the same content. Convention: `README.md` + `README.pt-BR.md`, `docs/<name>.md` + `docs/<name>.pt-BR.md`. This applies to the README, DECISIONS, CHANGELOG, guides and this spec (save this one as `docs/spec.pt-BR.md` and create the translation `docs/spec.md`). When changing one, update the other in the same commit.
- The message examples in this spec are already in English; the Portuguese names here are only explanation.

---

## 1. Overview

tokred is a single Rust binary, cross-platform (Windows, Linux and macOS). It runs **alongside rtk** (rust token killer) and reduces cost and token consumption in Claude Code:

1. **Model routing:** installs subagents with a fixed model (opus, sonnet or haiku) and hooks that suggest and enforce the correct delegation.
2. **Context guard:** hooks that prevent redundant or useless reads (re-reading an unchanged file, lockfiles, generated files, huge files without `offset`/`limit`).
3. **Audit:** measures the weight of the fixed context (CLAUDE.md, skills, agents, configured MCPs × usage).
4. **Metrics:** reads Claude Code transcripts and shows tokens, cost, routing savings, guard savings and projection.
5. **Local store:** SQLite in tokred's folder, with a tiered cache (L1/L2/L3) for session state and artifacts.

### Non-goals (for now)

- **LLM response cache / HTTP proxy:** out of the MVP. Hooks do not intercept API calls, agentic prompts almost never repeat byte for byte, and Anthropic already does prefix caching on the server. It will be reassessed in phase 7, based on the metrics data.
- Compressing Bash command output: that is rtk's job.
- Redis or any external service.

---

## 2. Principles (non-negotiable)

1. **Never overwrite user files wholesale.** tokred only adds, updates or removes **its own parts** (section 5).
2. **Idempotency:** running `configure` or `update` N times produces the same result.
3. **Fail-open:** if a hook errors, panics or times out, it **exits with code 0 and no output**, and logs the error to a file. tokred must never hang or block Claude Code because of its own bug. Use `std::panic::catch_unwind` in the hooks' entrypoint.
4. **Latency:** each hook execution must stay under ~50 ms (p95). No network inside a hook.
5. **Local and private:** no data leaves the machine. The only network calls are the price sync (section 9) and the version check (section 14), made only from CLI commands, never in hooks.
6. **Minimal injected context:** any `additionalContext` is at most one or two lines long, and is only emitted when something changes.

---

## 3. Architecture

Clean architecture in idiomatic Rust style: **pure business rules at the center, I/O at the edges**, dependencies always pointing inward, and a single composition point (the binary).

### 3.1 Repository layout

```
tokred/
├── Cargo.toml                    workspace: members, [workspace.package], [workspace.dependencies], [workspace.lints]
├── Cargo.lock                    versioned (project with a binary)
├── rust-toolchain.toml           stable channel + rustfmt + clippy
├── rustfmt.toml
├── clippy.toml
├── deny.toml                     cargo-deny: licenses, advisories, sources
├── dist-workspace.toml           dist configuration (releases)
├── .editorconfig
├── .gitattributes                line-ending normalization (LF; *.ps1 with CRLF)
├── .gitignore
├── LICENSE-MIT / LICENSE-APACHE
├── README.md / README.pt-BR.md
├── CHANGELOG.md / CHANGELOG.pt-BR.md
├── CONTRIBUTING.md / CONTRIBUTING.pt-BR.md
├── .github/
│   ├── workflows/ci.yml          fmt, clippy, tests (3 OSes), cargo-deny, coverage
│   ├── workflows/release.yml     generated by dist + publishing to winget and scoop
│   ├── dependabot.yml            cargo + github-actions
│   ├── ISSUE_TEMPLATE/           bug and feature
│   └── pull_request_template.md
├── crates/
│   ├── tokred-core/              domain: types, errors, config model, path resolution, traits (ports)
│   ├── tokred-install/           use case: configure/unconfigure, settings merge, agents, manifest
│   ├── tokred-hooks/             use case: hook handlers (route, enforce, guard-read, guard-search, session-*)
│   ├── tokred-metrics/           use case: transcript ingestion, cost, savings, projection, latency
│   ├── tokred-pricing/           use case: price table, sync, embedded fallback
│   ├── tokred-audit/             use case: fixed-context audit
│   ├── tokred-update/            use case: version check, install method detection, self-update
│   ├── tokred-report/            metrics HTML generation (template + SVG)
│   ├── tokred-store/             adapter: SQLite (rusqlite), migrations, repository implementations
│   └── tokred-cli/               `tokred` binary: clap, adapter composition, output (table/JSON)
├── templates/agents/             .md templates for the subagents (English)
├── tests/fixtures/               real settings.json files (with rtk hooks), hook stdin, JSONL transcripts
└── docs/                         bilingual documentation (section 16)
```

### 3.2 Layers and dependency rules

```
tokred-cli  ──►  use cases (install, hooks, metrics, pricing, audit, update, report)  ──►  tokred-core
     │                                                                                          ▲
     └──►  adapters (tokred-store, http/fs modules)  ── implement the traits of ───────────────┘
```

- **`tokred-core`** does no I/O: domain types, errors (`thiserror`), config model, and the **traits (ports)** that the use cases consume: `Clock`, `FileSystem`, `SessionRepo`, `UsageRepo`, `CacheRepo`, `PriceSource`, `ReleaseSource`, `HookEventSink`, etc.
- **Use cases** depend only on `tokred-core`. The main logic is made of **pure functions** over data, for example:
  - `merge_settings(current: Value, entries: &[HookEntry]) -> Result<Value, MergeError>`
  - `classify(prompt: &str, rules: &Rules) -> TaskClass`
  - `next_tier(entry: &CacheEntry, now: Timestamp, policy: &TierPolicy) -> Tier`
  - `cost(usage: &Usage, prices: &PriceTable) -> Money`
  
  I/O comes in only through the traits, received as parameters (generics or `&dyn Trait`).
- **Adapters** (`tokred-store`, and the `http` and `fs` modules inside the crates that need them) implement the traits. No use case imports `rusqlite`, `ureq` or `std::fs` directly.
- **`tokred-cli`** is the only composition point: it builds the real adapters, calls the use cases, formats the output and decides the exit code. Only it uses `anyhow`.
- `Clock` exists so that cache TTLs, the 24 h window of the version check and the per-day metrics are testable without waiting for time to pass.

### 3.3 Code standards

- Rust 2024 edition, `rust-version` (MSRV) declared in the workspace and verified in CI.
- Dependency versions centralized in `[workspace.dependencies]`; crates use `dep = { workspace = true }`.
- Lints in `[workspace.lints]`: `unsafe_code = "forbid"`; clippy `pedantic` as `warn`; `unwrap_used`, `expect_used`, `panic`, `todo` and `dbg_macro` as `deny` outside tests.
- Errors: `thiserror` with crate-specific enums; no `String` as an error; `anyhow` only in the binary.
- Public items with doc comments (`///`), and each crate with a module doc (`//!`) explaining its role in the architecture. `cargo doc --no-deps` with no warnings.
- Minimal public API: `pub(crate)` by default, expose only what another crate uses.
- Logs with `tracing`; never `println!` outside `tokred-cli`.
- Formatting with `cargo fmt`; commits following Conventional Commits (`feat:`, `fix:`, `docs:`...), in English.

### 3.4 Tests

| Type | Where | What it covers |
|---|---|---|
| **Unit** | `#[cfg(test)] mod tests` inside each module | Pure functions: merge, classifier, cache tier policy, cost and projection calculation, install method detection, transcript parsing |
| **Per-crate integration** | `crates/<crate>/tests/` | Use case with real adapters in a temporary directory: `tokred-store` with real SQLite in `tempfile`, `tokred-install` against `settings.json` fixtures, `tokred-metrics` against fixture transcripts |
| **End to end (CLI)** | `crates/tokred-cli/tests/` | Runs the real binary with `assert_cmd` + `predicates`: `configure` → hooks with fixture stdin → `stats`/`metrics` → `unconfigure`, all inside a temporary `--claude-dir` and data folder |
| **Snapshot** | alongside the tests above | `insta` for the `settings.json` resulting from the merge, the `--json` output of the commands and the metrics HTML |
| **Property** | merge tests | `proptest`: configure twice = configure once; configure + unconfigure = original |

Rules:
- **No test touches the real `~/.claude` or data folder.** The `TOKRED_DATA_DIR` variable and the `--claude-dir` flag exist for this purpose too; tests always use temporary directories.
- No test accesses the network: `PriceSource` and `ReleaseSource` have fake implementations for testing.
- Tests run on the 3 OSes in CI, including Windows paths with spaces.
- Coverage measured with `cargo llvm-cov`; target of at least 80% in the use case crates and in `tokred-core`.
- Every bug fix comes with a test that reproduces the bug.

### 3.5 Suggested crates

`clap` (derive), `serde`, `serde_json` (feature **`preserve_order`**), `toml`, `rusqlite` (feature **`bundled`**), `directories`, `blake3`, `globset`, `walkdir`, `thiserror`, `anyhow` (binary only), `ureq` with rustls (http adapters only), `comfy-table`, `time`, `opener`, `tracing` + `tracing-appender`.

For tests: `tempfile`, `assert_cmd`, `predicates`, `insta`, `proptest`.

### Paths

| Item | Source |
|---|---|
| Claude Code folder | `$CLAUDE_CONFIG_DIR` if set; otherwise `~/.claude` (on Windows, `%USERPROFILE%\.claude`) |
| tokred data folder | `TOKRED_DATA_DIR` if set; otherwise `directories::ProjectDirs` → Linux `~/.local/share/tokred`, macOS `~/Library/Application Support/tokred`, Windows `%APPDATA%\tokred` |
| Binary | Installed by the package manager or script (section 14). tokred does not copy its own binary |
| Database | `<data>/tokred.db` |
| Config | `<data>/tokred.toml` |
| Logs | `<data>/logs/` (daily rotation, keep 7 days) |

---

## 4. CLI commands

| Command | Function |
|---|---|
| `tokred configure [--claude-dir <path>] [--yes]` | Detects the Claude Code folder (or asks), creates the database and the config, merges the settings, installs the agents and syncs prices (section 5) |
| `tokred update` | Updates the binary according to the install method (section 14) and idempotently reapplies the configuration |
| `tokred unconfigure [--purge]` | Removes only tokred's parts from Claude Code; `--purge` also deletes the data folder. The binary itself is removed by the package manager |
| `tokred doctor [--bench]` | Shows what is installed, versions, outdated files, conflicts with other hooks and whether rtk was detected; `--bench` measures the real latency of each hook (section 10) |
| `tokred stats [--since 7d] [--by model\|day\|project]` | Tokens, real cost, counterfactual cost, savings and projection |
| `tokred stats latency [--since 7d] [--by hook\|day\|session]` | How much time tokred's hooks added to Claude Code |
| `tokred metrics [--since 30d] [--no-open]` | Generates the visual HTML report (section 10), always in the same file, and opens it in the browser |
| `tokred audit [--project <path>]` | Estimated weight of the fixed context and unused MCPs |
| `tokred pricing sync` / `pricing show` | Updates or shows the price table |
| `tokred cache stats` / `cache prune` | Cache state per tier and cleanup of expired entries |
| `tokred hook <event>` | **Internal**, called by Claude Code. Reads JSON from stdin and writes JSON to stdout |

All display commands accept `--json` for machine-readable output.

---

## 5. `tokred configure`: detection and non-destructive merge

Installing the binary (section 14) **does not touch anything** in Claude Code. The integration only happens when the user runs `tokred configure`.

### 5.0 `configure` flow

1. **Find the Claude Code folder**, in this order:
   1. the `--claude-dir <path>` flag;
   2. the value saved in `tokred.toml` (`[claude] dir`) from a previous configuration;
   3. the `CLAUDE_CONFIG_DIR` variable;
   4. the default `~/.claude` (Windows: `%USERPROFILE%\.claude`).
   
   A folder is valid if it exists and has `settings.json` or `projects/`.
2. **If not found:** in an interactive terminal, asks for the path, validates it and asks again if it is invalid. Without an interactive terminal (CI, script), exits with an error suggesting `--claude-dir`.
3. Saves the chosen path to `tokred.toml`.
4. Detects rtk and reports that the two will coexist.
5. **Shows the plan** before applying: hooks that will be added or updated in `settings.json`, agents that will be created or updated, `.new` files due to conflicts. Asks for confirmation; `--yes` skips the question.
6. Creates the data folder, the database (migrations) and the config (merge, section 13), applies the settings merge (5.2), installs the agents (5.3) and syncs prices (if it fails, uses the embedded fallback).
7. Ends by showing the next steps (`tokred doctor`, `tokred metrics`).

### 5.1 Signature

Every tokred hook entry has a `command` that contains the marker **`tokred hook `**. Example:

```json
{ "type": "command", "command": "\"/opt/homebrew/bin/tokred\" hook guard-read", "timeout": 5 }
```

- The binary path is always quoted, because on Windows it may contain spaces.
- **Use a stable path**, one that survives upgrades: the path through which the binary is found on `PATH` (e.g. `/opt/homebrew/bin/tokred`, the scoop shim, the winget link), and **not** the canonicalized path with the version in its name (e.g. `.../Cellar/tokred/0.3.2/bin/tokred`), which breaks on the next upgrade. Look up `tokred` on `PATH` and check that it points to the current executable; if it is not found, use `current_exe()` and warn in `doctor`.
- `doctor` checks that the path recorded in the settings still exists; `configure` fixes it.

### 5.2 `settings.json` merge algorithm

1. If the file does not exist, create `{}`.
2. Read and parse it. **If the JSON is invalid: abort with a clear error, without writing anything.**
3. Back it up to `settings.json.tokred.bak` (keep only the latest).
4. For each event in `hooks`: walk through the groups (matcher + `hooks[]`) and **remove only the entries whose `command` contains the marker**. If a group is left with an empty `hooks`, remove the group. If the event is left empty, remove the event.
5. Insert the current tokred entries (section 7), in their own groups. **Do not mix them into the same group as rtk or user hooks.**
6. All other keys (`permissions`, `env`, `model`, third-party hooks, etc.) stay intact and **in their original order**.
7. Write to a temporary file in the same directory and then do an atomic rename.

`unconfigure` is the same algorithm, without step 5.

### 5.3 Agents

- The files live in `<claude>/agents/`, with the `tokred-` prefix (e.g. `tokred-architect.md`).
- The first line of the body, right after the frontmatter, is the comment `<!-- managed-by: tokred vX.Y.Z -->`.
- The manifest (`installed_files` table) stores the path, the blake3 hash of the installed content and the version.
- On update, if the file's current hash **equals** the manifest's, it is overwritten. If it **differs** (the user edited it), it is not overwritten: it writes `tokred-x.md.new` next to it and warns in `doctor`. The `--force` flag overwrites anyway.
- Files without the prefix and without the marker are **never** touched.

### 5.4 Required merge tests

Fixtures with: empty settings; settings with rtk hooks; settings with user hooks on the same event; settings with an old tokred version; invalid JSON; unknown keys. For each one, confirm that configure → configure → unconfigure returns a file semantically equal to the original.

---

## 6. Subagents

| File | Model | Use |
|---|---|---|
| `tokred-architect` | opus | Architecture, design decisions, trade-offs |
| `tokred-planner` | opus | Planning and breaking down large tasks |
| `tokred-dev` | sonnet | Implementation, refactoring, bug fixing |
| `tokred-reviewer` | sonnet | Code and test review |
| `tokred-explorer` | haiku | Code search, locating files and symbols (read-only) |
| `tokred-summarizer` | haiku | Summarizing docs, logs and long outputs |

Frontmatter: `name`, `description` (when to delegate), `model`, `tools` (restrict to what is needed; explorer and summarizer are read-only).

**Common rules in the body of every agent** (they reduce output tokens and what flows back into the main context):
- Do not recap the task or describe the process.
- Targeted edits instead of rewriting whole files.
- **Fixed, short return format:**
  ```
  RESULT: <1-3 lines>
  FILES: <list of changed or relevant paths>
  PENDING: <list or "none">
  ```

---

## 7. Hooks

All hooks: read stdin, respect the configured mode (`off` | `warn` | `enforce`), record the event in `hook_events` and follow the fail-open rule.

### 7.1 `route` (UserPromptSubmit)

- **Heuristic classifier in Rust**, no LLM. It uses keywords in pt-BR and en (pt-BR examples: arquitetura, design, planejar, plano → architect/planner; implementar, corrigir, bug, refatorar → dev; revisar → reviewer; onde fica, encontrar, buscar → explorer; resumir → summarizer), the prompt length and file mentions.
- Output: a **one-line** `additionalContext`, emitted only if the class changed relative to the previous prompt in the same session. Example: `tokred: implementation task → delegate to the tokred-dev subagent if it involves more than one edit.`
- **Do not delegate small tasks:** short, one-off prompts get no suggestion.
- Records the classification in `classifications`.

### 7.2 `enforce` (PreToolUse, matcher on the subagent tool: confirm `Task`/`Agent` in the docs)

- Compares the chosen subagent with the session's last classification.
- `warn`: allows and returns a short reason. `enforce`: denies with a reason that names the correct agent.
- Non-tokred agents: always allow.

### 7.3 `guard-read` (PreToolUse, matcher `Read`)

1. **Default block** (configurable via globs): lockfiles (`package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `Cargo.lock`, `poetry.lock`), `*.min.js`, `*.map`, `dist/**`, `build/**`, `node_modules/**`, `target/**`, `.git/**`. Denies with a short reason.
2. **Large file:** above `max_read_bytes` (default 200 KB) and without `offset`/`limit`, denies and asks for a ranged read.
3. **Re-read:** computes the file's blake3. If `(session_id, path, hash)` already exists in `file_reads` with the same `offset`/`limit`, denies, saying the content has not changed and is already in context. **Exception:** if a compaction or `/clear` happened after the read (detected via the session event), allow.
4. Records the read and the bytes avoided (for the metrics).

### 7.4 `guard-search` (PreToolUse, matcher `Grep|Glob`)

- In a large repository (more than `large_repo_files` files, counted and cached per root), deny a search without `path` or without a restrictive pattern. The default mode is `warn`.

### 7.5 `session-check` (Stop)

- Reads `transcript_path` from stdin and takes the `usage` of the last response to estimate the context size.
- Above `context_warn_tokens` (default 120k), emits a **message to the user** (not to the model) suggesting `/compact` or `/clear` when the task is wrapped up. Never force continuation.

### 7.6 `session-start` (SessionStart)

- Registers the session. On `source=clear|compact`, marks a cut-off so that `guard-read` allows reads again.

### 7.7 Coexistence with rtk

- rtk acts on the `Bash` PreToolUse; tokred does not register a matcher on `Bash`.
- `doctor` detects rtk's hooks and confirms there is no overlap.

---

## 8. Store (SQLite)

Configuration: `journal_mode=WAL`, `busy_timeout=2000`, versioned migrations (`user_version`).

```sql
sessions(id TEXT PK, project TEXT, started_at INT, last_seen INT, last_reset_at INT)
classifications(session_id TEXT, ts INT, class TEXT, prompt_len INT)
file_reads(session_id TEXT, path TEXT, hash TEXT, offset INT, lim INT, bytes INT, ts INT,
           PRIMARY KEY(session_id, path, hash, offset, lim))
hook_events(ts INT, session_id TEXT, hook TEXT, decision TEXT, reason TEXT, bytes_saved INT,
            duration_us INT, outcome TEXT)   -- outcome: ok | error | panic | timeout_guard
cache_entries(key TEXT PK, kind TEXT, value BLOB, tier INT, hits INT,
              created_at INT, last_hit INT, expires_at INT NULL)
installed_files(path TEXT PK, hash TEXT, version TEXT, installed_at INT)
pricing(model TEXT PK, input REAL, output REAL, cache_write REAL, cache_read REAL,
        source TEXT, updated_at INT)            -- USD per million tokens
usage(message_key TEXT PK, session_id TEXT, project TEXT, ts INT, model TEXT,
      input INT, output INT, cache_write INT, cache_read INT, is_subagent INT)
ingest_state(file TEXT PK, offset INT, mtime INT)
```

### Tiered cache policy (`cache_entries`)

| Tier | Enters when | TTL |
|---|---|---|
| L1 | First write | `l1_ttl` (default 4h) |
| L2 | `hits >= l2_hits` (default 3) | `l2_ttl` (default 24h), renewed on every hit |
| L3 | `hits >= l3_hits` (default 10) | Persistent (`expires_at = NULL`) |

- The key always includes the hash of the source content, so invalidation is automatic when the file changes.
- Promotion happens on read (increments `hits`, re-evaluates the tier).
- `cache prune` removes expired entries. It also runs opportunistically in `stats` and `update`, never inside a hook.
- In the MVP, the cache stores: file count per root, classifications and the audit result. Artifact summaries are left for phase 6.

---

## 9. Pricing

- Remote source: LiteLLM's public price JSON (`model_prices_and_context_window.json` in the BerriAI/litellm repository). Filter the Claude models and normalize to USD per million tokens.
- **Fallback embedded** in the binary (`include_str!`), generated at build time, for when there is no network.
- The sync runs on `configure`, `update`, `pricing sync` and on the daily sync (section 14), and records `source` and `updated_at`.
- Unknown model in the transcript: map by family (opus/sonnet/haiku) to the family's most recent price and flag it in `stats`.

---

## 10. Metrics

### Ingestion

- Read `<claude>/projects/**/*.jsonl` incrementally (`ingest_state` stores each file's offset and mtime).
- Extract from the assistant messages: model, `input_tokens`, `output_tokens`, `cache_creation_input_tokens` and `cache_read_input_tokens`. **Confirm the field names in a real transcript before implementing.**
- Deduplicate by message id + request id, because streaming can produce repeated lines.
- Mark `is_subagent` when the line comes from a subagent.

### Calculations in `stats`

- **Real cost:** sum per model × price.
- **"All on opus" counterfactual:** the same tokens at opus pricing. The difference is the **routing savings**.
- **Guard savings:** `bytes_saved / 4` as an estimate of avoided tokens (always label it as an estimate) × the session model's input price.
- **Prompt cache rate:** `cache_read / (cache_read + cache_write + input)`.
- **Projection:** daily average of the last 7 and 30 days × 30.
- **Before and after:** if there is data from before the first `configure` (the transcript is older), compare the cost per session before and after.

### Latency added by tokred (`tokred stats latency`)

Goal: show how much time tokred delayed Claude Code, since each hook runs synchronously before Claude continues.

**Measurement inside the binary**
- Capture `Instant::now()` on the first line of `main` and record `duration_us` at the end of each hook execution, including on error or panic (via a guard with `Drop`).
- Record `outcome` (`ok`, `error`, `panic`, `timeout_guard`).
- The database write is the last step, after emitting the response on stdout and closing stdout, so that the write itself is not added to the perceived delay. If the write fails, ignore it (fail-open).

**Process startup cost**
- The process spawn time by Claude Code is not visible from inside the binary. `tokred doctor --bench` measures it by running each hook N times (default 50) with stdin fixtures and timing from the outside (spawn → exit).
- The bench result (average spawn overhead per OS) is saved in the database and added to the internal durations in `stats latency`, labeled as an estimate.

**What `stats latency` shows**
- Per hook: number of executions, p50, p95, p99 and maximum in ms.
- Total ms added per session and per day, and average per user prompt.
- Count of `error`, `panic` and `timeout_guard`.
- Alert when the p95 of any hook exceeds the 50 ms budget.
- Optional comparison: ms added × tokens and cost saved in the same period, to show whether the trade-off is worth it.

### HTML report (`tokred metrics`)

**File**
- Fixed path: `<data>/reports/metrics.html`. Each run **overwrites the same file** (writes to a temporary file and does an atomic rename). Never generates files with a date in the name.
- Opens in the default browser with the `opener` crate, unless `--no-open` is passed. Prints the path to the terminal.

**Construction**
- **Self-contained, offline** HTML: inline CSS and JS, no CDN, no external fonts, no network requests.
- The template is embedded in the binary (`include_str!`). The data goes in as a JSON block (`<script type="application/json" id="data">`) generated from the database.
- Charts as **SVG generated in Rust** (bars, lines and donut), with no JS chart library. Inline JS only for tooltips and for switching the period.
- Light and dark themes via `prefers-color-scheme`, responsive layout, system font.
- Footer with the generation date, the tokred version, the period covered and the date of the last price sync.

**Sections**
1. **Summary cards:** real cost, total savings (routing + guards), tokens avoided (estimate), monthly projection and total latency added.
2. **Cost per day:** line of real cost × "all on opus" counterfactual.
3. **Distribution by model:** donut with tokens and cost for opus, sonnet and haiku.
4. **Routing:** classifications by task type, correct delegations × those corrected by `enforce`.
5. **Guards:** blocks by reason (re-read, generated file, large file, broad search) and bytes/tokens avoided.
6. **Prompt cache:** hit rate per day.
7. **Latency:** p50/p95/maximum per hook, with the 50 ms budget line, and the ms added × savings comparison.
8. **Audit:** the heaviest items of the fixed context and the unused MCPs.

Sections without data (for example, before the corresponding phase exists) appear with a "no data yet" notice instead of disappearing.

---

## 11. Audit (`tokred audit`)

Token estimate = characters / 4 (labeled as an estimate).

- The user's and the project's CLAUDE.md (including files imported with `@`).
- Installed skills and agents (frontmatter and body).
- **MCPs:** list the configured servers (settings and the project's `~/.claude.json`; confirm the location in the docs) × actual usage in the transcripts (`tool_use` with name `mcp__<server>__*`). Point out servers with no use in the last N days. The exact weight of the schemas cannot be computed locally; show only usage.
- Output: table sorted by weight, with a short suggested action.

---

## 12. Phases

| Phase | Deliverable | Done when |
|---|---|---|
| F0 | Repository bootstrap (section 15), workspace with the crates from section 3, paths, config, logging, store with migrations, `configure`/`unconfigure`/`doctor` with folder detection, non-destructive merge and agents; README and docs in both languages (section 16) | CI green on the 3 OSes (fmt, clippy, unit, integration and end-to-end tests, cargo-deny); merge tests (5.4) passing |
| F1 | Release pipeline (`dist`, brew tap, scoop bucket, winget publishing), `update` by install method, pricing (sync + fallback + daily sync), daily version check with notice, `stats` with incremental ingestion and the first version of `tokred metrics` (sections 1, 2, 3 and 6) | `stats` matches the manual sum of a test transcript; the HTML opens offline and is overwritten on every run. The remaining HTML sections arrive together with the phases that generate the data |
| F2 | `audit` | Lists the context weight and the unused MCPs |
| F3 | `guard-read`, `guard-search`, `session-start`, latency measurement, `stats latency` and `doctor --bench` | Stdin fixtures covering allow, deny and the cut-off after compaction; `doctor --bench` showing p95 < 50 ms |
| F4 | `route` + `enforce` | Classifier with a table of pt-BR/en test cases |
| F5 | `session-check` | Warning fires above the limit and never forces continuation |
| F6 | Artifact cache (summaries by hash) | Only after measuring F1 to F5 |
| F7 | Evaluate proxy/response cache | Decision recorded in DECISIONS.md based on the data |

---

## 13. Config (`tokred.toml`)

```toml
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

The user's config also goes through a merge on update: new keys are added with the default value and existing values are preserved.

---

## 14. Distribution

Installing and configuring are **two separate steps**:

```sh
brew install sostenes198/tap/tokred   # 1. installs the CLI (does not touch Claude Code)
tokred configure                  # 2. integrates with Claude Code
```

### Installation channels

| OS | Command | How it is published |
|---|---|---|
| macOS and Linux | `brew install sostenes198/tap/tokred` | Formula in the `sostenes198/homebrew-tap` tap, generated by `dist` on every stable release |
| Windows | `winget install tokred` | Manifest submitted to `winget-pkgs` on every release by a publishing action in CI (`dist` does not generate winget natively; confirm in the current docs) |
| Windows | `scoop bucket add sostenes198 https://github.com/sostenes198/scoop-bucket` + `scoop install sostenes198/tokred` | Manifest in the `sostenes198/scoop-bucket` bucket, updated by CI |
| Any | Shell script (`curl … \| sh`) or PowerShell (`irm … \| iex`) | Generated by `dist` |
| Any | `cargo install tokred` | crates.io |

- Build with `dist` (formerly cargo-dist) for `x86_64`/`aarch64` on Linux (musl), macOS and Windows, with checksums on every release.
- **No channel touches the Claude Code folder during installation.** The brew formula shows in its caveats: `Run "tokred configure" to set up the Claude Code integration.` The scripts print the same message at the end.

### Updating according to the install method

tokred detects how it was installed from the path of its own executable:

| Method | Detection (path contains) | What `tokred update` does |
|---|---|---|
| Homebrew | `Cellar` or `linuxbrew` | Shows `brew upgrade tokred` (does not update itself) |
| winget | `WinGet\Packages` or `WinGet\Links` | Shows `winget upgrade tokred` |
| scoop | `scoop\apps` or `scoop\shims` | Shows `scoop update tokred` |
| cargo | `.cargo/bin` | Shows `cargo install tokred --force` |
| Script | anything else | Performs the self-update (download, checksum, atomic swap) |

- After the binary changes version, the first execution of a **CLI command** compares the current version with the one recorded in the database and, if it changed, reapplies the configuration non-interactively (hooks and agents). Hooks do not do this; until then they keep working with the previous configuration, and `doctor` shows that a reconfiguration is pending.
- CI (GitHub Actions) with a matrix of the 3 OSes running the tests, including the merge with Windows paths.

### New version check

- **Once a day**, on the first execution of a CLI command (`stats`, `metrics`, `audit`, `doctor`, `cache`, `pricing`), query the latest published release (GitHub releases API).
- **Never inside a hook.** The `tokred hook` subcommand does no check and no network access.
- The result and the time of the last check are stored in the database. Within 24h, it does not query again, it just reuses the saved result.
- 2 s timeout. With no network or on error: silently ignore and try again the next day.
- If there is a newer version, print **to stderr**, after the command's output, a line with the right command for the install method, for example `new version 0.4.0 available (current 0.3.2) → run: brew upgrade tokred`. Since it goes to stderr, it does not break `--json`.
- Self-update (script installation only): downloads the release binary, **verifies the checksum**, replaces it atomically and reapplies the configuration. On Windows, renames the old `.exe` to `.old` before copying the new one and deletes the `.old` on the next run.
- Config: `[update] mode = "notify"` (default) | `"off"` | `"auto"`. `auto` only applies to script installation; with package managers it behaves like `notify`. Even in `auto`, it only runs from CLI commands, never from a hook. The `TOKRED_NO_UPDATE_CHECK=1` environment variable disables the check (useful in CI).

### Daily price sync

- Same rule as the version check: at most once a day, only from CLI commands, with a short timeout and silent failure.
- It is automatic by default, because it is only data and not executable code. It can be disabled with `[pricing] auto_sync = false`.

---

## 15. Repository bootstrap (create from scratch)

The repository https://github.com/sostenes198/tokred starts empty. First step of F0:

1. `git init`, default branch `main`, remote `origin` pointing to `git@github.com:sostenes198/tokred.git`.
2. Create the workspace (root `Cargo.toml`) with all the crates from section 3.1, each with a `Cargo.toml` inheriting `[workspace.package]` (version, edition, rust-version, license, repository, authors) and `lints.workspace = true`. Use case crates start with a `lib.rs` containing the module doc and a trivial test, so CI is green from the first commit.
3. **`.gitignore`** with the Rust standard and related entries:
   ```gitignore
   # Rust
   /target/
   **/*.rs.bk
   *.pdb

   # Coverage / profiling
   *.profraw
   *.profdata
   lcov.info
   /coverage/

   # insta (snapshots pending review)
   *.snap.new
   *.pending-snap

   # Local data / databases created by tests or manual runs
   *.db
   *.db-shm
   *.db-wal
   /reports/

   # Editors / IDEs
   .idea/
   .vscode/*
   !.vscode/extensions.json
   *.swp
   *.swo
   *~

   # OS
   .DS_Store
   Thumbs.db
   desktop.ini

   # Env
   .env
   .env.*
   ```
   `Cargo.lock` **is versioned** (the project has a binary).
4. **`.gitattributes`:** `* text=auto eol=lf`, `*.ps1 text eol=crlf`, binaries marked as `binary`.
5. **`.editorconfig`:** UTF-8, LF, 4 spaces in `.rs`, 2 in `.toml`, `.yml`, `.json` and `.md`, newline at end of file.
6. **`rust-toolchain.toml`:** `stable` channel, `rustfmt` and `clippy` components.
7. Minimal **`rustfmt.toml`** and **`clippy.toml`** (e.g. `max_width = 100`; `msrv` equal to the workspace's).
8. **`deny.toml`:** allowed licenses (MIT, Apache-2.0, BSD, ISC, Unicode, Zlib), advisories as errors, crates.io as the only source.
9. **License:** the Rust ecosystem's suggested default, `MIT OR Apache-2.0` (two `LICENSE-*` files). **Confirm with the author before creating.**
10. **CI (`.github/workflows/ci.yml`):** on push and PR to `main`; jobs for `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --workspace` in a Linux/macOS/Windows matrix, MSRV check, `cargo deny check`, `cargo doc --no-deps` with no warnings and coverage with `cargo llvm-cov` (report as an artifact).
11. Weekly **`dependabot.yml`** for `cargo` and `github-actions`.
12. Issue templates (bug, feature) and PR template, in English, with a link to `CONTRIBUTING`.
13. `dist` (`dist init`) is configured in F1, together with the release pipeline.
14. First commit: `chore: bootstrap workspace`.

---

## 16. Documentation

All documentation is bilingual (section 0): the English version under the base name and the Portuguese version with the `.pt-BR.md` suffix. Each file starts with the language selector:

```markdown
**English** | [Português](README.pt-BR.md)
```

### 16.1 `README.md` (entry point)

Short and to the point; details live in the docs and the README points to them. Sections, in this order:

1. Name, short description and badges (CI, release, license, crates.io).
2. **What it is** and why it exists (2 or 3 paragraphs, mentioning rtk).
3. **Features** as a short list, each item linking to the corresponding doc.
4. **Installation** per OS (brew, winget, scoop, script, cargo) → link to `docs/installation.md`.
5. **Quick start:** `tokred configure`, `tokred doctor`, `tokred metrics`.
6. **Commands** in a summary table → link to `docs/commands.md`.
7. **How it works** in one paragraph → link to `docs/architecture.md` and `docs/hooks.md`.
8. **Configuration** → link to `docs/configuration.md`.
9. **Privacy:** everything local, the only network calls and how to turn them off.
10. **Uninstallation:** `tokred unconfigure` + the package manager command.
11. **Contributing** → `CONTRIBUTING.md`; **License**.

### 16.2 `docs/`

| File (+ `.pt-BR.md`) | Content |
|---|---|
| `docs/installation.md` | All installation channels, updating by method, version check, uninstallation |
| `docs/commands.md` | Reference for each command, flags, examples and `--json` output |
| `docs/configuration.md` | Each `tokred.toml` key, default values and environment variables (`TOKRED_DATA_DIR`, `TOKRED_NO_UPDATE_CHECK`, `CLAUDE_CONFIG_DIR`) |
| `docs/architecture.md` | Layers, crates, dependency rules and the flow of a hook, with a diagram |
| `docs/hooks.md` | Each hook: event, what it decides, `off`/`warn`/`enforce` modes, how it coexists with rtk |
| `docs/agents.md` | The subagents, each one's model and the return format |
| `docs/metrics.md` | How each number is calculated, what is an estimate, latency and the HTML report |
| `docs/cache.md` | SQLite store and the L1/L2/L3 policy |
| `docs/development.md` | Requirements, build, running tests (unit, integration, end-to-end, snapshots), coverage, how to test hooks locally with fixtures, how to cut a release |
| `docs/decisions.md` | Decision log (date, context, decision, consequences) |
| `docs/spec.md` | This specification |

At the root: `CONTRIBUTING.md` (branch and PR flow, Conventional Commits, test and bilingual docs checklist) and `CHANGELOG.md` (Keep a Changelog format, updated on every release).

Rules:
- Command examples in the docs must work as written; those in the quick start section are covered by an end-to-end test.
- When changing behavior, the PR updates the corresponding doc in both languages.
