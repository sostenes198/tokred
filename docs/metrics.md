**English** | [Português](metrics.pt-BR.md)

# Metrics

What `tokred stats`, `tokred stats latency` and `tokred metrics` show, where the numbers come from, and which of them are estimates.

| Command | Shows |
|---|---|
| `tokred stats` | Tokens, cost, savings, cache hit rate and monthly projection, in the terminal |
| `tokred stats latency` | The time the hooks added to Claude Code |
| `tokred metrics` | The same terminal report (30 days by default), plus a self-contained HTML file opened in the browser |

Flags are in [commands](commands.md). `--json` on `stats` prints `{"period": ..., "stats": {...}}` (with a `proxy` key when the proxy recorded something).

## Sources

**Transcripts.** tokred reads Claude Code's session files, `<claude>/projects/**/*.jsonl`, incrementally into its database: each run resumes where the last one stopped, a line Claude Code is still writing is left for the next run, and a message that streaming repeats is counted once. From each assistant message it takes the model, the four token counters (input, output, cache write, cache read) with the split of cache writes between 5 minutes and 1 hour, the session, the project and whether it came from a subagent. Malformed lines and messages without usage are skipped.

**The proxy.** When the optional [proxy](proxy.md) ran, it recorded its own exact data for every request: the model asked for and the model that answered, tokens, time to first byte and total time, the likely cause of a prompt-cache miss, the origin (main loop, subagent, auxiliary, compaction), the agent type of a subagent and the time the proxy itself added.

**Merged, never counted twice.** Every proxied request is also in the transcripts, so the two are merged before anything is computed:

- A proxy request is paired with the transcript message that has the same message id and request id. A proxy request without an id (an interrupted stream) is paired with a message of the same session whose four counters are equal and whose time falls between 5 seconds before the request arrived and 5 seconds after its response ended.
- A pair becomes one row with the proxy's numbers: the model that really answered, the exact token split, the origin and the agent type. The transcript contributes the project.
- Rows seen by only one side stay: transcript-only rows (the proxy was off) and proxy-only rows. A request answered with an error status or with no tokens is not a message.

Without the proxy, every number is computed from the transcripts alone.

## What `tokred stats` computes

Totals, groups, savings and the cache rate cover the period given by `--since` (default `7d`). The projection and the before/after comparison also look outside it, on purpose.

| Number | How it is computed | Estimate? |
|---|---|---|
| Totals | Messages, sessions, the four token counters and cost over the period | No |
| Real cost | Sum over messages of tokens times price (USD per million tokens). A model with no price costs 0 and is listed as unpriced | No |
| Everything on opus | The same tokens at the latest opus price (messages of a model with no price are left out). The difference to the real cost is the **routing saving** | Yes: what routing changed is measured, the opus price is a counterfactual |
| Guard saving | Each denial by a `guard-*` hook counts `bytes avoided / 4` tokens at the input price of the model the session used most. Only denials count: in `warn` mode the read still happens | Yes, and labelled as such |
| Prompt cache hit rate | `cache read / (cache read + cache write + input)`, overall and per day | No |
| Monthly projection | Average daily cost of the last 7 days and of the last 30 days, times 30. A data set younger than the window is divided by its real number of days, so a new install is not diluted | Yes |
| Before and after | Cost per session before and after the first `tokred configure`; shown only when there is data from before it | No |

Anything derived from bytes is labelled an estimate. Money is rounded to 6 decimals, and an empty period never produces a NaN.

### Groupings

`--by` regroups the rows:

| `--by` | One row per |
|---|---|
| `model` | Model |
| `day` | Day (UTC) |
| `project` | Project folder |
| `origin` | Who sent the request: `main`, `subagent`, `auxiliary`, `compaction` |
| `agent-type` | Agent type of a subagent request (`Explore`, `Plan`, ...) |

Without the proxy, `origin` falls back to the transcripts (`main` or `subagent`) and every agent type is `(unknown)`.

### The Proxy section

When the proxy recorded something in the period, `stats` adds a **Proxy** section:

| Line | What it shows |
|---|---|
| Data source | Messages measured by the proxy and messages known only from the transcripts |
| Requests | Requests the proxy handled and how many the upstream did not answer with a 2xx status |
| Overhead | p50, p95, p99 and max of the time the proxy added, against its 10 ms budget; an alert when the p95 passes it |
| Routing | Decisions of the period (applied, observed, kept, fallback) with the **estimated** saving of the applied ones, and of the observed ones had they been applied |
| Model routing | The model asked for against the model that answered, per pair: messages, tokens and cost |
| Cache misses | How many main-loop messages missed the prompt cache out of all of them, then a table by likely cause with the tokens each cause wrote to the cache again. A session can interleave several kinds of request that the proxy sees as the main loop (the conversation and side queries that reuse its context), each with its own cache, so every request is compared with the previous one of its own series (same model and same system prompt); the first request of a series counts as a miss, with the cause `model_changed` or `system_changed` |
| Suggestion | The latest cheaper-model suggestion not shown yet (see `tokred suggest`); `stats` never marks it as shown |

## Latency added by tokred

Hooks run before Claude Code continues, so `tokred stats latency` reports for each hook: runs, p50, p95, p99 and max in ms; the total and the average added per prompt; the outcomes (`ok`, `error`, `panic`, `timeout_guard`); and an alert when a hook's p95 passes 50 ms (exactly 50 is within budget). It also puts the total time added next to the tokens the guards avoided in the same period. Percentiles use the nearest-rank method.

A process cannot see its own start-up time, so `tokred doctor --bench` measures it from outside, saves it, and that value is added to every run, which makes the total **an estimate**. Until you run `doctor --bench` once, the start-up time counts as 0 and the figures are a lower bound.

## The HTML report

`tokred metrics` writes `<data>/reports/metrics.html` (always the same file, overwritten) and opens it (`--no-open` only writes it). It is one file with inline styles, scripts and charts, so it works offline and respects your light or dark theme. A section without data says "no data yet". The proxy's sections (model routing, overhead, cache misses, cost by origin and agent type) appear only when the proxy recorded something.

The **7 days / 30 days / All** buttons switch the summary cards (real cost, savings, tokens avoided, latency) and the daily charts together; each view says which days it covers, and All is the whole `--since` period (30 days unless you pass another). The projection of the 7 days and All views comes from the last 7 days, and the 30 days view's from the last 30. The other sections (models, routing, guards, latency, audit) always cover the whole period.

## Prices

Prices come from the public [LiteLLM](https://github.com/BerriAI/litellm) price list, kept in tokred's database. `tokred pricing sync` refreshes them; other commands refresh them at most once a day (turn it off with `[pricing] auto_sync = false`, or set `TOKRED_OFFLINE=1`). If the list cannot be reached, the previous table stays, and a table that is empty is seeded from a snapshot embedded in the binary. Cache writes of 1 hour are priced at their own, higher, rate.

A model that is not in the table is priced at the latest price of its family (opus, sonnet, haiku, fable or mythos) and flagged in the report; one that belongs to no family costs 0 and is listed as unpriced.
