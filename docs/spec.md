**English** | [Português](spec.pt-BR.md)

# tokred — Specification (v3, final)

> **Update 2026-10-29:** distribution changed. tokred is installed by scripts that download the GitHub release (rustup style); Homebrew, Scoop, winget and crates.io are no longer used, and `configure` now runs at the end of the install. See the 2026-10-29 entries in [decisions](decisions.md).

**Repository:** https://github.com/sostenes198/tokred

**Short description (GitHub "About"):** *Token and cost reducer for Claude Code — local proxy with model policy, context guards and metrics. Works alongside rtk.*

**Description:** tokred is a local, cross-platform Rust CLI that reduces the cost and token consumption of Claude Code. It has two complementary pieces: **hooks** that keep useless content out of the context, and a **local proxy** that sits between Claude Code and the Anthropic API to measure every request exactly, choose the model for auxiliary tasks and subagents, and suggest to the user when a cheaper model would be enough. Everything runs on the user's machine, with its own SQLite, with no external service. It runs alongside rtk (rust token killer), which compresses command output.

> **This is the final version of the spec.** It replaces v1 and the v2 draft (both in [`docs/archive/`](archive/)). It states **what has already been done**, **what changes** and **what is missing** for the project to work as a proxy. The parts already implemented are defined by the documents in `docs/` (section 2); here they appear in summary form.

---

## 0. Instructions for Claude Code

- Before implementing **each hook**, consult the official hooks documentation (https://code.claude.com/docs/en/hooks). Before implementing **any part of the proxy**, also consult the gateway compatibility guide (https://code.claude.com/docs/en/llm-gateway-protocol.md), the prompt caching page and the API's *preserved thinking* page. **Do not assume field or header names.** Where this spec diverges from the documentation, the documentation wins: record the divergence in `docs/decisions.md`.
- Implement in phases (section 12). At the end of each phase: tests passing on all 3 operating systems, `cargo clippy -- -D warnings` clean, line coverage maintained and a summary of what was done.
- Ask before any decision that changes the behavior described here.
- **Never make more than one `git push` per user request**, and never without explicit authorization.

### Language rules

- **Code in English:** folders, files, crates, modules, types, functions, variables, tables and columns, config keys, comments, commit messages and all strings displayed by the CLI, the hooks and the proxy.
- **Documentation always in both languages** (`X.md` + `X.pt-BR.md`, with the same content), updated in the same commit. Documents in `docs/archive/` are an exception (history).
- The classifier keywords remain in pt-BR and en (they are data).

---

## 1. Vision and what changed since v1

### 1.1 The problem

The cost of a Claude Code session is dominated by **context resent on every turn** (read from the cache at 0.1× the input price, or rewritten at 1.25× when the cache misses) and by the **model** used in each request. Hooks can keep useless content out of the context, but they **do not see the request to the model**: they do not know exactly what was billed, do not know why the cache missed and do not decide which model serves each call.

### 1.2 The new idea

A **local proxy** (`tokred proxy`) sees every request and every response. This gives three things that hooks do not:

1. **Exact measurement** (usage per request, per request type, per subagent, per prompt) and **cache miss explanation**.
2. **Model policy** for what the user did not choose directly (inherited subagents, auxiliary tasks).
3. **Grounded suggestion** of a cheaper model for the main loop, without ever switching it on its own.

### 1.3 What the official documentation establishes (verified)

These facts shape the entire spec; they were read on the official Claude Code and API pages:

1. **Claude Code identifies each request for gateways.** Headers `x-claude-code-request-class` (`main`, `subagent`, `workflow`, `compaction`, `auxiliary`), `x-claude-code-session-id`, `x-claude-code-agent-id`, `x-claude-code-parent-agent-id`, `x-claude-code-agent-type` (`Explore`, `Plan`, `general-purpose`, `custom`, `teammate`, `fork`), `x-claude-code-prompt-id`, `x-claude-code-compaction`, `x-claude-code-context-compacted` and `x-claude-code-prev-tool-durations`. The proxy classifies traffic **without interpreting the body**. The headers are an open list: treat them as such. **Verified in the F7 spike** (Claude Code 2.1.312, `-p` mode, API key): only `x-claude-code-session-id` (all requests) and `x-claude-code-agent-id` (subagents only) arrived; `request-class`, `agent-type` and `prompt-id` did **not** appear. Classification therefore uses the presence of `agent-id`, the tool set and the system blocks, and treats the other headers as a bonus when present (also checked in interactive mode: `request-class` does not arrive). An **auxiliary request** is recognized by its shape: no `agent-id`, no tools, no `thinking` and exactly one message (e.g. the session title); the main loop always has tools and can never match.
2. **Gateways must "inspect without modifying".** Body fields and `anthropic-beta` headers come in pairs; rewriting the body breaks the pair and produces `400`. The body and the `anthropic-*` headers must be forwarded without filtering by a fixed list.
3. **Preserved thinking.** The API rejects (`400`, "bound to a different conversation") when the earlier `system`, `tools` or `messages` differ from what existed when the thinking block was produced. A gateway that rewrites that content can cause the rejection. Switching the `model` between turns does **not** violate this check, but blocks that the new model cannot read are dropped without error.
4. **Prompt cache.** The cache is by identical prefix (tools → system → messages) and breaks when anything earlier changes. Default TTL of 5 minutes (1 hour optional); a write costs 1.25× (5 min) or 2× (1 h) the input price; a read costs 0.1× (0.05× on Opus 5.5 and Sonnet 5.5, 0.025× on Fable 5.1 and Mythos 5.1). Minimum cacheable sizes vary from 512 to 4,096 tokens per model. Switching models in the main loop rewrites the entire context.
5. **Subscription.** Setting only `ANTHROPIC_BASE_URL` keeps the claude.ai login active; the gateway must forward the OAuth value of `anthropic-beta`. Anthropic does not support routing Claude Code to non-Claude models.
6. **Auxiliary requests** (titles, classifiers, summaries) use, depending on the login type, the main model or Haiku. Some (next-prompt suggestion) **reuse the conversation cache**.
7. **Claude Code already offers model controls without a proxy:** `ANTHROPIC_DEFAULT_*_MODEL`, `CLAUDE_CODE_SUBAGENT_MODEL`, `model:` in the subagent frontmatter, `opusplan`, effort levels. The proxy adds per-request enforcement, complete measurement (including subagents) and visibility; it is **not** the only way to route.
8. **New models appear frequently** (Fable and Mythos families, different cache multipliers per model). Nothing about models can be hardcoded: it must come from versioned, updatable data.

### 1.4 Decision changes relative to v2

| v2 | v3 |
|---|---|
| Proxy does active `tool_result` compression | Only in **shadow mode** (computes what it would save, changes nothing). Turning it on for real requires the phase F10 gate (section 12) |
| Proxy without model routing | **Model policy** for subagents and auxiliary tasks; main loop **untouched** |
| `route` hook as the main classifier | Claude Code headers replace the keyword classifier **for measuring**; `route` becomes an optional suggestion |
| Response cache (phase F8) | **Shadow** only (measures repetition by hash); active cache goes to the backlog |
| Artifact cache (F9) | Backlog |
| No reversible compression | **Reversible compression** of large `tool_result` blocks (phase F11): the proxy deterministically replaces the block with a preview plus an id, and the model gets the original back through the `tokred_retrieve` tool of the `tokred mcp` MCP server (section 6.7). Starts in `observe` |

### 1.5 Non-goals

- Compressing Bash output in the hook (rtk's role) and depending on `rtk-proxy`.
- Switching the **main loop** model (only suggest).
- Routing to non-Claude models.
- Semantic cache, Redis or any external service.
- Modifying `system`, `tools`, `thinking`, `cache_control`, images or thinking blocks.

---

## 2. Current state: what has already been done

Everything below is on `main` (published repository). The exact behaviors are in the indicated documents and in the records in `docs/decisions.md`.

### 2.1 Delivered (phases F0 to F5 of v1)

| Component | State | Where it is defined |
|---|---|---|
| 10-crate workspace, CI, `cargo-deny`, MSRV 1.89, bilingual docs | Done | `docs/development.md`, `docs/architecture.md` |
| `configure` / `unconfigure` / `doctor` with non-destructive merge, `tokred-*` agents and manifest | Done | `docs/commands.md`, `docs/agents.md` |
| Six fail-open hooks: `guard-read`, `guard-search`, `session-start`, `route`, `enforce`, `session-check`, with latency recording | Done | `docs/hooks.md` |
| SQLite store (schema v2, migrations), layered L1/L2/L3 cache | Done | `docs/cache.md` |
| Pricing: LiteLLM list, embedded snapshot, daily sync | Done | `docs/metrics.md`, `docs/configuration.md` |
| Metrics: incremental transcript ingestion, cost, savings, projection, latency | Done | `docs/metrics.md` |
| Colored terminal dashboards (`stats`, `stats latency`, `metrics`) and offline HTML report | Done | `docs/commands.md` |
| `audit` (fixed context and unused MCPs), `cache`, `pricing`, `update` with verified self-update, `doctor --bench` | Done | `docs/commands.md` |
| Release pipeline (`dist`, Homebrew, Scoop, winget, crates.io) _(superseded 2026-10-29: install scripts and GitHub release only, see [decisions](decisions.md))_ | Written, **never executed** | `docs/installation.md` |
| Quality | 655 tests; 100% of lines covered; fmt, clippy, doc, deny and MSRV clean on macOS | `docs/development.md` |

### 2.2 Known problems (to fix before the proxy)

1. **CI on Windows is red.** The test `ingestion_resumes_and_waits_for_partial_lines` (`tokred-metrics`) fails: with the file open for writing, the size read through directory metadata is stale on Windows. In addition, `cargo test --workspace` stops at the first binary that fails, so **other Windows failures may be hidden** (paths, globs with `\`, end-to-end tests). Fix: get the size from the file handle (`File::open(..).metadata()`), run CI with `--no-fail-fast` and handle whatever shows up.
2. **Prices and model families.** `Family` only knows opus, sonnet and haiku; Fable and Mythos are missing. The cache read multiplier is derived as 0.1× when it is missing from the data, which is wrong for new models (0.05× and 0.025×). The 1-hour write price (2×) is missing.
3. **The release pipeline has not been exercised** (it needs 4 secrets and a tag; the first winget manifest is manual). _Superseded 2026-10-29: the pipeline now needs only a tag, see [decisions](decisions.md)._
4. **`route` uses keywords** and not the request class headers (which only the proxy sees).

---

## 3. Principles (non-negotiable)

1. **Never overwrite user files entirely** (only the tokred parts). `configure`/`update` are idempotent.
2. **Fail-open everywhere.** Hook: exit 0 with no output. Proxy: any error in a transformation or decision → forward the **original** request intact; a panic is isolated per request; the daemon stays up. tokred must never prevent Claude Code from working because of its own bug.
3. **Latency.** Hook: p95 < 50 ms. Proxy: p95 overhead < 10 ms to the first byte (measured, section 8.4). Hooks never access the network.
4. **Local and private.** Nothing leaves the machine beyond what Claude Code already sends to Anthropic. The proxy forwards **only** to the configured host (default `https://api.anthropic.com`). Credentials and request bodies **never** go to the log or to the database (only hashes and counts).
5. **Inspect without modifying by default.** The proxy only changes a request in explicitly allowed cases (section 6) and always with an escape hatch. The default mode of any feature that changes traffic is `observe`.
6. **Minimal injected context:** any `additionalContext` or `systemMessage` is one or two lines and only appears when something changes.
7. **Data-driven decisions.** A feature that changes traffic is only turned on after `observe`/shadow mode has shown real gain and no regression (section 12, gates).

---

## 4. Architecture

Clean architecture in idiomatic Rust style: **pure business rules at the center, I/O at the edges**, dependencies always pointing inward and a single composition point (the binary). The v1 rules still apply (strict lints, `thiserror`, `anyhow` only in the binary, `pub(crate)` by default, `tracing`, `&dyn` instead of generics where it helps coverage).

### 4.1 Crates

```
crates/
├── tokred-core/      pure domain + ports. NEW: RequestClass, AgentType, TaskClass, Tier,
│                     ModelCapabilities, RoutingDecision, cost estimation with cache state
├── tokred-install/   configure/unconfigure/doctor, settings merge, agents
├── tokred-hooks/     hook handlers (synchronous, no tokio)
├── tokred-metrics/   transcript ingestion, cost, savings, projection, latency
├── tokred-pricing/   prices, sync, embedded snapshot. NEW: model capabilities table
├── tokred-audit/     fixed context audit
├── tokred-update/    version, install method, self-update
├── tokred-report/    HTML report
├── tokred-store/     SQLite adapter, migrations, repositories
├── tokred-proxy/     NEW. Asynchronous HTTP daemon: streaming forwarding, SSE tap,
│                     classification, routing, measurement, kill switch
└── tokred-cli/       `tokred` binary: composition, output, commands
```

### 4.2 Proxy-specific rules

- **Async boundary.** `tokio`/`hyper` live **only** in `tokred-proxy` and only start in the `proxy` subcommand. Hooks and the other commands remain synchronous and free of that startup cost.
- **Pure logic in `tokred-core`/use cases:** `classify_request`, `decide_route`, `estimate_cost`, `explain_cache_miss`, `summarize_prompt`, `suggest_model` are pure functions testable without a network. `tokred-proxy` only does I/O and calls these functions.
- **Single SQLite writer.** The proxy serves concurrent requests, but SQLite has one writer: requests send records through a channel (`mpsc`) to a task that writes in batches. A write failure never affects the response to the client.
- **New ports** in `tokred-core`: `ProxyRepo` (requests, decisions, prompt summaries, suggestions), `ModelCatalog` (capabilities, families, tiers), `UpstreamClient` (for fakes in tests).
- **New dependencies** (only in `tokred-proxy`): `tokio`, `hyper` + `hyper-util` (or `axum`), `reqwest` with rustls and streaming, `bytes`; in tests, `wiremock` and our own fake SSE server.

### 4.3 Paths

Unchanged from v1 (Claude Code folder, data folder via `BaseDirs`, database, config, logs). New: `<data>/proxy/` with the admin secret file (section 6.8) and the PID.

---

## 5. Model policy

This is the heart of v3. Principle: **respect what the user chose; decide only what they did not choose; suggest the rest.**

### 5.1 Who decides what

| Request class (`x-claude-code-request-class`) | Can the proxy switch the model? | How |
|---|---|---|
| `main` (main loop) | **Never** (and no configuration option allows it) | Only measures and **suggests** (section 5.6) |
| `subagent`, `workflow` | Yes, **once per agent** | Decision on the first request of each `agent-id`, fixed until it ends (section 5.3) |
| `auxiliary` | Yes, **per request** and only if it pays off | Cost calculation with cache state (section 5.4) |
| `compaction` | Not by default | Summary quality affects the rest of the session; opt-in via configuration |

### 5.2 Classification layers (the first one that decides wins)

Each layer returns `(task_class, confidence, reason)`; the engine only acts above a confidence threshold (`min_confidence`). When in doubt, it **keeps the requested model**.

1. **Headers (deterministic).** `request-class` and `agent-type`. E.g.: `Explore` → search (cheap class); `Plan` → planning (keep); `auxiliary` → auxiliary.
2. **Explicit user choice.** If the subagent's model **differs** from the model the main loop of the same session is using, someone set the model on purpose (frontmatter, variable, `opusplan`): **respect it**. A decision is only made when the subagent **inherited** the main model.
3. **Tool set in the request (objective).** A subagent with only read tools (`Read`, `Grep`, `Glob`, searches) changes nothing: low risk for a cheaper model. The presence of `Edit`/`Write`/`Bash` raises the risk.
4. **Keywords in the task prompt** (the subagent's first message), in pt-BR and en, reusing the `route` classifier (moved to `tokred-core`): find/locate/summarize → cheap; implement/fix/refactor → mid; architecture/design/plan/in-depth review → keep.
5. **Default:** keep.

### 5.3 Subagents: sticky decision

- The decision is recorded per `agent-id` (`routing_decisions`) and **reused** in all of that agent's following requests. It is never reclassified mid-execution: that would break the cache (each subagent has its own) and thinking continuity.
- Target: the tier defined for the class (`[routing.tiers]`, section 10), resolved to a concrete model by the catalog (section 7.3). **Downgrade only:** the target can never cost more than the requested model, nor be outside `allowed_models`.

### 5.4 Auxiliary requests: cost-aware routing

Some auxiliary requests reuse the conversation cache (the next-prompt suggestion is "almost only cache read"). Moving one of them to Haiku **loses the cache** and pays a write at 1.25×, which can cost **more** than the read at 0.1× on the expensive model. Therefore:

- Route only when `estimate_cost(target, no cache) < estimate_cost(original, known cache state) × (1 − margin)`.
- Known cache state: the proxy stores, per session, the hash of the main prefix and the instant of the last read/write; if the auxiliary request's prefix matches and the cache is warm, **do not route**.
- In practice: titles, classifiers and small independent summaries are routed; suggestions that carry the entire context are not.

### 5.5 Parameter compatibility (mandatory before switching the model)

Different models accept different parameters (adaptive thinking vs fixed budget, `effort`, interleaved thinking, `context_management`, 1M context). Switching the `model` without adjusting the rest can produce `400`. Rules:

- The catalog (section 7.3) holds the capabilities per model. **Route only** if all fields and betas present in the request are supported by the target. Otherwise, **keep** the model (no other field is rewritten).
- **Safety net:** if the rewritten request receives `400`/`404`/a model error before any response byte is sent to the client, resend **once** with the original body and model, record the reason and count the failure for the circuit breaker.
- **Circuit breaker:** above `max_rewrite_failure_rate` (default 5% over a window of 50 rewrites) routing is turned off for `cooldown` and `doctor` warns. There is also a manual switch (`tokred routing off`).

### 5.6 Main loop: suggest, never switch

The model configured by the user is always respected. tokred follows each user prompt and, when the task was simple, **suggests** a cheaper model with a summary of the evidence.

**Aggregation.** Requests with the same `x-claude-code-prompt-id` form a prompt. For each one the proxy records (`prompt_summaries`): number of main turns, tools used, output and thinking tokens, actual cost, whether there was an edit, subagents spawned and tool durations.

**"Simple" (retrospective, transparent).** Configurable, documented criteria, for example: few turns, small output, low thinking, read-only or a small edit. Retrospective is more reliable than predicting; the prediction for the next prompt uses only the pattern of the last N.

**Counterfactual.** Cost of the same prompt on the appropriate cheaper tier, using the catalog price and **the cost of rebuilding the cache** when there is a switch: `saving = actual_cost − (target_cost + cache_rewrite)`. It only suggests if `saving > minimum`.

**When and how to notify.** Never interrupt the model. Three channels, all optional (`[suggest] channel`):
1. `tokred stats`/`metrics`: "Suggestions" section.
2. **Status line** (`tokred statusline`): a discreet line ("simple over the last 5 prompts · Sonnet would save ~R$/US$ x"). `configure` only writes the `statusLine` key if it is empty and asks first.
3. **`UserPromptSubmit` hook** with `systemMessage` (a message for the user, not for the model), **only at safe boundaries**: cache already cold (idle beyond the TTL), right after `/clear` or compaction (free switch), at most once per session and per time window.

The hook only **reads** the suggestion from the database (no network, within the 50 ms budget).

### 5.7 Auditing and transparency

- Every decision is recorded with its reason and `rules_version`. `tokred routing explain <request-id|agent-id>` shows the layer, confidence, cost estimate and outcome.
- `observe` mode (default): records "would have routed X → Y" **without changing** anything, and shows the hypothetical savings. Only `enforce` changes traffic.
- Each feature has a mode `off | observe | enforce`.

---

## 6. The proxy

Local HTTP daemon between Claude Code and the API. Optional. Independent of `rtk-proxy`.

```
Claude Code ──(ANTHROPIC_BASE_URL=http://127.0.0.1:1029)──► tokred proxy ──► api.anthropic.com
                                                              │
                                                              ├─ reads class/session/agent headers
                                                              ├─ classifies and (if enforce) chooses the model
                                                              ├─ SSE tap: measures usage without delaying delivery
                                                              └─ records request, decision and prompt
```

### 6.1 Forwarding

- Listens on loopback only (`127.0.0.1`), default port `1029`; refuses any other address without an explicit `--bind`. **Validates the `Host` header** (defense against DNS rebinding) and responds `421` for hosts that are not loopback.
- Forwards only to `upstream` (HTTPS, rustls). All `anthropic-*` and `x-claude-code-*` headers, `Authorization`/`x-api-key` and the rest pass **unchanged** (no fixed list). Credentials **never** go to the log or the database.
- Only `POST /v1/messages` goes through the policy. Every other endpoint (`count_tokens`, `/v1/models`, etc.) is forwarded intact.
- **Streaming:** the response is forwarded event by event, without buffering; the proxy observes the events in parallel. `ping` events and the `content-type: text/event-stream` header pass as they came (Claude Code has a stream idle watchdog). API errors (4xx, 5xx, 429) and their bodies, and the rate limit headers, are forwarded **exactly**.
- The upstream timeout is the client's; the proxy never shortens it.

### 6.2 Fail-open

An error or panic in classification, decision or measurement → forward the original body and log it. A database error → ignore (the response was already delivered). If the proxy is down, `tokred claude` opens Claude Code without the proxy and warns.

### 6.3 How Claude Code points to the proxy

- **Default: `tokred claude [args…]`.** Checks `GET /__tokred/health`, starts the proxy if needed and opens Claude Code with `ANTHROPIC_BASE_URL` set **only for that run**. Nothing is written to `settings.json`.
- **Optional: `persist_base_url = true`.** `configure` writes `env.ANTHROPIC_BASE_URL` through the merge in section 5 of v1. Risk: with the proxy stopped, all of Claude Code stops. Therefore it requires the proxy as a user service (`tokred proxy service install`: systemd user unit, LaunchAgent, scheduled task at logon) and `session-start` tries to bring the proxy up. `configure` explains the risk and asks for confirmation. If an `ANTHROPIC_BASE_URL` with another value already exists, it **does not overwrite**: it shows it and asks. `unconfigure` removes only the tokred value.
- Authentication: the proxy does **not** replace credentials. With an API key (`x-api-key`) or a subscription (`authorization`, verified in the F7 spike), it forwards what it receives (the subscription requires forwarding the OAuth value of `anthropic-beta`).

### 6.4 Measurement (per request)

Records in `proxy_requests`: requested model and effective model, `usage` (input, output, cache write, cache read, thinking), class, agent type and id, session and prompt id, body bytes, time to the upstream's first byte, total time, **proxy overhead** (time spent before sending), status, `routing_mode`, `routing_decision_id`, cache TTL used.

### 6.5 Cache miss explanation

For each `main` request with low `cache_read`, the proxy explains the likely reason by comparing with the previous one in the same session: **idle time above the TTL** (interval between requests), **compaction** (`compaction`/`context-compacted` headers), **tool change** (hash of the `tools` block), **system change** (hash of the `system`), **effort change** (thinking/effort fields), or **unknown**. Only segment hashes are stored. Claude Code also shows a "likely cause" in `/usage`; tokred also covers subagents and aggregates over time.

### 6.6 Compression and response cache shadow

Without changing anything, the proxy computes and records:
- **Compression (shadow):** it would apply the reversible compression of section 6.7 (preview + id, original kept in the database) to text `tool_result` blocks and record the bytes and tokens that **would have** been avoided, per tool. **There is never lossy truncation:** all replaced content stays retrievable through `tokred_retrieve`. Gain estimate: a block of T tokens costs ≈ `1.25·T` on write and `0.1·T` per following turn; the report shows the expected gain already accounting for the observed number of turns.
- **Response cache (shadow):** canonical hash (blake3) of the body to measure how many requests are **exact repeats**. Only hashes and counts.

### 6.7 Reversible compression of tool results

**Principle: no information is lost.** tokred never truncates content without keeping the original; everything removed from a request can be retrieved.

Goal: the model does not pay, on every turn, for a huge `tool_result` it has already used (e.g. a 5,000-line JSON), but can get it back when it needs it. Inspired by the equivalent mechanism of `rtk-proxy`, with a deliberate difference in how the content is retrieved.

**How it works**
1. **Selection.** Only **text** `tool_result` blocks above `min_tokens` (default 1,500, estimated as bytes / 4). Never small source-file `Read` results, images, documents, `tool_use`, `system`, `tools`, `thinking` or `cache_control`.
2. **Deterministic replacement.** The proxy stores the original (`stored_results`, compressed blob, `retention`, default 7 days) and sends instead a preview (the first `preview_head_lines` and last `preview_tail_lines` lines) followed by a one-line marker: `[tokred: N lines omitted (id=<id>). Call tokred_retrieve with this id and an optional offset/limit to read them.]`. The `id` is derived from the hash of the original content.
3. **Same bytes on every turn.** The replacement happens **the first time** the block is sent to the model (the turn right after the tool ran), so the model **never** saw the original in that context. On later turns Claude Code resends the original and the proxy reapplies exactly the same replacement (map `(session, hash of the original) → bytes sent`). This keeps the cache prefix and the thinking blocks (*preserved thinking*) consistent. If the map disagrees with the current computation, the map wins.
4. **Visible retrieval.** `configure` registers a local MCP server (`tokred mcp`, stdio) with the tool `tokred_retrieve(id, offset?, limit?)`. The model calls it like any other tool, Claude Code runs it, and the answer enters the history normally.

**Why not inject a hidden tool (like `rtk_retrieve`)**
- Answering the tool inside the proxy hides an exchange from Claude Code. On the next turn the resent history lacks that exchange, and on models with *preserved thinking* (Opus 5.5, Sonnet 5.5, Haiku 5.5, Fable 5.1) the API compares the prefix and may reject with `400` ("bound to a different conversation") or silently drop thinking blocks.
- Injecting a tool changes the `tools` block, which invalidates the whole cache if it happens mid-session.
- With the MCP server, the tool exists from the start of the session (MCP tools load on demand) and every retrieval is recorded in the transcript.

**Safeguards**
- Mode `off | observe | enforce` (default `observe`: only measures how much it would save, section 6.6). `enforce` only after gate G2.
- If the proxy goes down mid-session, Claude Code starts sending the original: one cache rebuild and possible loss of thinking blocks; `doctor` and the metrics show it.
- Retrieving the same `id` more than `max_retrievals` times in a session signals that the preview is too short for that tool: the tool is then excluded from compression automatically.
- Circuit breaker: any binding/thinking `400` or a drop in the cache rate after enabling turns compression off immediately and records why.
- **Future extension (not included):** replacing repeated file bodies in `Write`/`Edit` arguments with a pointer (`rtk-proxy`'s `retire-pointer`). It touches the assistant's `tool_use`; evaluate after measuring F11.

### 6.8 Administration and security

- `/__tokred/*` endpoints (health, status, stop) require a **local secret** (a header with a random value written to `<data>/proxy/secret`, permission 0600) to prevent web pages or other processes from controlling the daemon.
- `log_bodies = false` by default. When it is off nothing about message contents is read, held or stored. When a user turns it on, the proxy captures only per-turn deltas (the messages new since the previous request, and the reassembled response), truncated, for the dashboard (`docs/dashboard.md`); it never captures headers or credentials. The trade-off is recorded in `docs/decisions.md` (2026-10-29).
- `doctor` shows: proxy running, port, service installed, current value of `ANTHROPIC_BASE_URL`, modes of each feature, circuit breaker state, Claude Code version seen in the headers.

### 6.9 Compatibility

- The official guide says the gateway must keep up with Claude Code versions. Hence the rule is to **forward everything that is not understood**; act only on what is known and tested.
- Compatibility matrix tested and recorded: login by **API key** and by **subscription**, `ANTHROPIC_BASE_URL` with and without `ANTHROPIC_AUTH_TOKEN`, models with and without preserved thinking, with and without 1M context.
- Check Anthropic's terms of use for subscription traffic through a proxy; if it is not allowed, document the proxy as "API key only". Record in `docs/decisions.md`.

---

## 7. Store and model catalog

### 7.1 Migration v3

Keeps all the v1/v2 tables of the schema (`sessions`, `classifications`, `file_reads`, `hook_events`, `cache_entries`, `installed_files`, `pricing`, `usage`, `ingest_state`, `meta`). New ones:

```sql
proxy_requests(id TEXT PK, ts INT, session_id TEXT, agent_id TEXT NULL, agent_type TEXT NULL,
               prompt_id TEXT NULL, request_class TEXT, model_requested TEXT, model_effective TEXT,
               input INT, output INT, thinking INT, cache_write INT, cache_read INT, cache_ttl TEXT,
               bytes_in INT, upstream_ttfb_ms INT, total_ms INT, proxy_overhead_us INT,
               status INT, routing_mode TEXT, decision_id TEXT NULL, miss_cause TEXT NULL,
               prefix_hash TEXT, tools_hash TEXT, system_hash TEXT)
routing_decisions(id TEXT PK, ts INT, session_id TEXT, agent_id TEXT NULL, scope TEXT,   -- agent|request
                  task_class TEXT, confidence REAL, layer TEXT, reason TEXT,
                  model_from TEXT, model_to TEXT, mode TEXT, est_cost_from REAL, est_cost_to REAL,
                  rules_version TEXT, outcome TEXT)                                      -- applied|observed|kept|fallback
prompt_summaries(prompt_id TEXT PK, session_id TEXT, ts INT, turns INT, tools TEXT,
                 output_tokens INT, thinking_tokens INT, cost_usd REAL, edited INT,
                 subagents INT, simple INT, est_saving_usd REAL)
suggestions(id TEXT PK, ts INT, session_id TEXT, kind TEXT, message TEXT, shown INT, accepted INT NULL)
shadow_savings(ts INT, session_id TEXT, kind TEXT, key TEXT, bytes_saved INT, est_tokens_saved INT, rules_version TEXT)
compressed_blocks(...)   -- only if active compression is turned on (F10+)
```

Indexes on `ts`, `session_id`, `prompt_id`, `agent_id`. Configurable retention (`[proxy] retention_days`, default 90) with opportunistic cleanup in CLI commands.

### 7.2 Metrics sources

When there is proxy data for the period, it is the primary source of usage and cost; the transcripts fill in what the proxy did not see. Deduplicate by message id. `stats`/`metrics` gain groupings by request class, agent type and prompt.

### 7.3 Model catalog (data, not code)

An embedded, updatable file (`models.json`, generated by a script from LiteLLM and the documentation, with an offline fallback) with, per model: family, prices (input, output, 5 min write, 1 h write, cache read), **capabilities** (adaptive thinking/budget, `effort`, interleaved, `context_management`, 1M context, minimum cacheable size), and the default **tiers** (cheap/mid/strong) per family. `Family` stops being a closed enum (it now accepts unknown families). Unknown model: family inferred from the name, **conservative** capabilities (do not route) and a warning in `stats`.

---

## 8. Changes to existing components

| Component | Change |
|---|---|
| `tokred-core` | Proxy and policy types; open `Family`; `ModelPrice` with 1 h write and per-model multipliers; `estimate_cost` with cache state; move the keyword classifier from `tokred-hooks` to here |
| `tokred-pricing` | Read the 1 h price and the real multipliers from LiteLLM (without deriving 0.1× when there is data); capabilities catalog; update script |
| `tokred-hooks` | `route` becomes an optional suggestion; new suggestion behavior in `UserPromptSubmit` (reads from the database, only at safe boundaries); `session-start` tries to bring the proxy up when configured; guards unchanged |
| `tokred-metrics` | Proxy + transcripts source; cost per class/agent/prompt; **real** routing savings (separate from the "everything on opus" counterfactual); cache miss explanations; shadow savings; accepted suggestion rate |
| `tokred-report` | New sections: proxy (p50/p95 overhead, 10 ms budget), routing decisions, suggestions, cache miss causes, shadow savings |
| `tokred-store` | Migration v3 and proxy repositories; single batched writer |
| `tokred-install` | `configure` asks about the proxy, `statusLine` and `ANTHROPIC_BASE_URL` (non-destructive merge, never overwrites someone else's value); `unconfigure` removes only what belongs to tokred |
| `tokred-update` | After updating the binary, restart the proxy daemon; schema compatibility |
| `tokred-cli` | New commands (section 9); `doctor` with the proxy section; `stats --by request-class\|agent-type\|prompt` |
| CI/Release | 3-OS matrix also for the proxy (tests with fake API and streaming); `--no-fail-fast`; per-OS user service installers |

---

## 9. Commands (new and changed)

| Command | Function |
|---|---|
| `tokred proxy start\|stop\|status` | Controls the daemon (`start` in the foreground or in the background) |
| `tokred proxy service install\|uninstall` | User service (systemd user unit, LaunchAgent, logon task) |
| `tokred claude [args…]` | Ensures the proxy and opens Claude Code pointed at it only for that run |
| `tokred routing status\|on\|off\|explain <id>` | Mode, circuit breaker, latest decisions and explanation of a decision |
| `tokred suggest [--since 7d]` | Model suggestions for the main loop, with the evidence |
| `tokred statusline` | Short line for Claude Code's `statusLine` |
| `tokred mcp` | **Internal.** MCP server (stdio) with the `tokred_retrieve` tool, registered by `configure` when compression is enabled (section 6.7) |
| `tokred stats … --by request-class\|agent-type\|prompt` | New groupings |
| `tokred doctor` | Proxy section (section 6.8) |

The other commands (`configure`, `unconfigure`, `update`, `audit`, `pricing`, `cache`, `metrics`, `stats`, `hook`) follow `docs/commands.md`.

---

## 10. Config (`tokred.toml`) — new keys

```toml
[proxy]
enabled = false
bind = "127.0.0.1:1029"
upstream = "https://api.anthropic.com"
persist_base_url = false         # writes env.ANTHROPIC_BASE_URL to settings (requires service)
retention_days = 90
log_bodies = false

[routing]
mode = "observe"                 # off | observe | enforce
min_confidence = 0.8
min_saving_usd = 0.005
max_rewrite_failure_rate = 0.05
cooldown = "30m"
allowed_models = []              # empty = any catalog model that is cheaper
compaction = "keep"              # keep | cheap (opt-in)

[routing.tiers]                  # task class -> tier; the catalog resolves to a concrete model
search = "cheap"
summary = "cheap"
implementation = "mid"
planning = "keep"
architecture = "keep"
review = "keep"

[suggest]
channel = "stats"                # off | stats | statusline | message
min_simple_prompts = 3
cooldown = "6h"

[compression]                    # reversible compression (section 6.7)
mode = "observe"                 # off | observe | enforce
min_tokens = 1500
preview_head_lines = 40
preview_tail_lines = 20
retention = "7d"
max_retrievals = 3

[shadow]
compression = true               # computes what it would save (changes nothing)
response_cache = true            # measures exact repeats by hash
```

The v1 keys for hooks, prices, update, cache and guards remain (see `docs/configuration.md`). The user's config continues to be merged on update.

---

## 11. Tests (proxy)

| Type | What it covers |
|---|---|
| **Unit** | Layered classification (table of pt-BR/en cases and headers), route decision, parameter compatibility, cost estimation with and without cache, cache miss explanation, prompt aggregation, "simple" rules |
| **Fake API** | Fake Anthropic server (`wiremock` + own SSE): byte-by-byte forwarding, event-by-event streaming, intact 4xx/5xx/429 errors, upstream dropping mid-stream, preserved headers (including unknown ones) |
| **Routing** | Sticky decision per `agent-id`; downgrade only; explicit choice respected; fallback resending the original on `400`; circuit breaker opens and closes; `observe` mode never changes a byte |
| **Property** | In `observe`/shadow the body sent to the upstream is **identical** to the one received; same input → same decision |
| **Load and latency** | p95 overhead < 10 ms to the first byte; 50 simultaneous streams; no memory leak |
| **End to end** | `tokred claude` with a fake "claude"; simulated multi-turn session |
| **3 OSes** | All of the above in CI (including Windows paths) |
| **Manual, recorded** | Real Claude Code with an API key and with a subscription (spike, F6); real Claude Code with routing in `enforce` in test sessions |

v1 rules kept: no test touches the real `~/.claude` or data folder; none accesses the external network; every bug fix comes with a test that reproduces it; **100% line coverage** (measured with `cargo llvm-cov`, reference: `lcov` and the annotated text).

---

## 12. Phases

Each phase ends with green tests on all 3 OSes, clean clippy, coverage maintained and documentation in both languages. The **gates** are data-based decisions, recorded in `docs/decisions.md`.

| Phase | Deliverable | Done when |
|---|---|---|
| **F0 to F5** | Done (section 2) | — |
| **F6 — Stabilization** | Fix the Windows CI (size from the handle; `--no-fail-fast`; handle whatever else shows up); model and price catalog (new families, multipliers, 1 h); exercise the release pipeline with a test tag | CI green on all 3 OSes; `stats` correct for new models; test release published |
| **F7 — Spike (≈ 1 day)** | Minimal forwarding proxy that only records headers and `usage`; test with an **API key** and with a **subscription**; confirm that the `x-claude-code-*` headers arrive; test rewriting the `model` of an auxiliary request | Result in `docs/decisions.md`: what works, what does not; **go/no-go** for the proxy and whether the subscription is supported |
| **F8 — Proxy: forwarding and measurement** | `tokred-proxy` (6.1 to 6.5, 6.8), `tokred claude`, `proxy start/stop/status/service`, migration v3, cache miss explanation, integration into metrics and `doctor` | Tests with fake API and streaming; p95 overhead < 10 ms; Claude Code normal with and without the proxy; no byte change in the traffic |
| **F9 — Model policy** | Layered classification, subagent and auxiliary routing in `observe` → `enforce` mode, capabilities catalog, fallback and circuit breaker, `routing explain` | In `observe`, hypothetical savings visible; **gate G1:** only turn on `enforce` if the rate of correct reviewed decisions is high and `observe` showed no quality losses; in `enforce`, 0 binding/thinking `400`s in test sessions |
| **F10 — Suggestions and shadow** | `prompt_summaries`, `suggest`, `statusline`, suggestion hook at safe boundaries; compression and response cache shadow; gain report | Suggestions with correct evidence in the tests; **gate G2:** active compression only if the shadow shows material gain (threshold defined in the data) **and** a canary with no `400` and no cache drop |
| **F11 — Reversible compression** | Section 6.7: deterministic replacement + `stored_results` + the `tokred mcp` MCP server with `tokred_retrieve`, in `observe` by default; MCP registration by `configure`; the active response cache stays in the backlog | Determinism and byte-for-byte prefix tests across turns; end-to-end retrieval with the real Claude Code; **gate G2** for `enforce`: gain measured in shadow mode and no `400` or cache drop in a canary |
| **Backlog** | Artifact cache (summaries by hash) and the other v1 ideas | Only with data |

---

## 13. Risks and mitigations

| Risk | Mitigation |
|---|---|
| The API or Claude Code change (new betas, fields, headers) | Forward everything that is not understood; act only on what is known and tested; compatibility matrix; circuit breaker; `doctor` shows the Claude Code version |
| Routing degrades response quality | Only low-risk tasks; inherited subagents; `observe` by default; confidence threshold; downgrade only; immediate `routing off`; sampled review in F9 |
| `400` from parameters incompatible between models | Capabilities catalog; only route if compatible; resend the original; circuit breaker |
| Losing the cache when switching models | Sticky decision per agent; auxiliary requests only when it pays off; never on the main loop |
| Preserved thinking / binding | Never alter `system`, `tools`, `messages` in `observe`; compression only in shadow until the gate |
| Proxy becomes a single point of failure | Fail-open; `tokred claude` without the proxy if it does not come up; service only with explicit opt-in |
| Local daemon security | Loopback, `Host` validation, admin secret, no logging of credentials or bodies |
| Terms of use for subscription via proxy | Check and record in the spike; if not allowed, "API key only" |
| Proxy maintenance cost | Minimal scope, data instead of code (catalog), tests with fake API, matrix in CI |
| Gain smaller than expected | Measure first (F8), shadow before acting (F10), gates G1/G2 with thresholds in the data |
| Reversible compression breaks the cache or thinking | Replacement the first time the block is sent and identical afterwards; retrieval through a visible MCP tool (no hidden exchange); circuit breaker; `observe` by default |
| The preview is not enough and the model does not retrieve | Explicit marker with the id and the tool; automatic exclusion of a tool with repeated retrievals; high threshold (`min_tokens`) |

### 13.1 Open questions (resolve in the F7 spike)

1. Does **subscription** traffic tolerate rewriting the `model` field (plans have per-family limits)? And is it allowed by the terms? **Technically answered (F7 spike):** a subscription login accepts a rewritten `model` on subagent requests (status 200, correct result). **Still open:** Anthropic's terms of use for subscription traffic through a proxy, and per-model plan limits.
2. Does Claude Code react to a response with a `model` different from the requested one (interface, internal accounting)? **Answered for an API key:** the API accepted it and Claude Code had no problem (F7 spike).
3. Do the `x-claude-code-*` headers appear on all surfaces (CLI, extensions)? **Answered for the CLI:** in both `-p` and interactive mode only `session-id` and `agent-id`.
4. Which `auxiliary` requests share the cache prefix in practice (measure)?

---

## 14. Distribution and documentation

- **Distribution:** unchanged (`dist`, Homebrew, Scoop, winget, crates.io; installing and configuring are separate steps) _(superseded 2026-10-29, see [decisions](decisions.md))_. New: the proxy as a user service is always **opt-in**; no installation channel activates the proxy.
- **Documentation:** remains bilingual. New: `docs/proxy.md` (how it works, security, compatibility), `docs/routing.md` (model policy and how to audit decisions), and updates to `commands`, `configuration`, `hooks`, `metrics`, `architecture`, `installation` and `decisions`. Command examples must work as written; those in the quick start section have an end-to-end test.
- **Privacy (README):** everything local; tokred's only network calls are price synchronization, the version check and `update` (only from CLI commands) and the proxy's forwarding to the API host; `TOKRED_OFFLINE=1` turns off the first three.
