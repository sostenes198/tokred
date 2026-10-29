**English** | [Português](metrics.pt-BR.md)

# Metrics

The numbers behind `tokred stats`, `tokred stats latency` and the HTML report come from the `tokred-metrics` crate: pure functions over data, plus one filesystem adapter. This page describes what they actually do, so the numbers mean what they say. See the [original design](archive/spec-v1.md) (section 10) and the [current spec](spec.md) for how the proxy extends it.

## Ingestion

`tokred-metrics` reads `<claude>/projects/**/*.jsonl` incrementally into the `usage` table.

- From each line with `type: "assistant"` it takes `sessionId`, `timestamp` (RFC 3339, truncated to seconds), `requestId`, `isSidechain` and `message.{id, model, usage}`, where `usage` holds `input_tokens`, `output_tokens`, `cache_creation_input_tokens` and `cache_read_input_tokens`. Every other line type (`user`, `attachment`, `queue-operation`, ...) is ignored.
- Messages whose model is `<synthetic>` and lines without `usage` are skipped.
- Streaming can repeat a message, so rows are deduplicated on `message_key = "<message.id>:<requestId>"`; the first line seen wins.
- `project` is the project directory name. A line is flagged `is_subagent` when it comes from a file under `<project>/<session>/subagents/` or has `isSidechain: true`. Subagent lines without `sessionId` take the session directory name.
- Malformed JSON lines are skipped, never fatal.

**Incremental reading.** `ingest_state` keeps, per file, the byte offset already consumed and the mtime. Each run resumes at the offset and only consumes complete lines: a trailing line without `\n` (Claude Code is still writing it) is left for the next run, so the saved offset always sits just after a newline. A file smaller than its saved offset was truncated or rotated and is read again from the start; because of the deduplication, re-reading never double-counts. The offset is saved only after the rows are stored. A file that is replaced by a larger one is not detected (there is no content hash); reading resumes at the old offset, so the first partial line is skipped as malformed and the bytes before the offset are not re-read.

Everything above is reached through the `TranscriptFs` port (list files with size and mtime, read bytes from an offset) and the `UsageRepo`/`IngestRepo` ports, so the whole pipeline is tested with temporary directories and no real `~/.claude`.

## What `tokred stats` computes

`compute_stats` receives **all** known usage rows, the price table, the hook events, a range, a grouping (`model`, `day` or `project`), `now` and, optionally, the first `configure` time. Totals, groups, savings and the cache rate only look at the range; the projection and the before/after comparison look outside it by design.

| Number | How |
|---|---|
| Totals and groups | Messages, distinct sessions, the four token counters and cost, for the range; groups ordered by key (day groups are UTC days) |
| Real cost | Sum over messages of tokens x price (USD per million tokens). A model without any price costs 0 and is listed in `unpriced_models` |
| Counterfactual "everything on opus" | The same tokens at the latest opus price; the difference is the **routing saving**. Messages of models with no price are left out so they cannot inflate it. `null` if the price table has no opus |
| Guard saving (**estimate**, `estimate: true`) | For every guard (`guard-*` hook) denial: `bytes_saved / 4` tokens (integer division per session) x the input price of the model that session used most (by message count, ties to the smaller model id; opus if the session is unknown or has no price) |
| Prompt cache rate | `cache_read / (cache_read + cache_write + input)`, overall and per UTC day; `null` when there was no input traffic |
| Projection | Daily average cost of the last 7 and of the last 30 days, x 30. The divisor is the window or, when the data is younger than the window, the days since the first row (at least 1), so a new install is not diluted |
| Before/after | Cost per session before vs. after the first `configure` (a session belongs to the side where it started). Only present when rows older than the first `configure` exist |
| `unpriced_models` | Models priced by **family fallback** (latest price of opus/sonnet/haiku) or **unresolvable**, with message and token counts, so the CLI can flag them |

Only denials count as guard savings: in `warn` mode the read still happens, so nothing was saved. Anything derived from bytes (`bytes / 4`) is always labelled an estimate. Money is rounded to 6 decimals; no ratio is ever NaN (an empty period gives `null`/0).

## Routing and guards summaries

- **Routing:** classifications per task class (the `decision` of `route` rows, ignoring `none`), most frequent first; delegations from `enforce` rows: `correct` (`agent_matches`), `corrected` (`agent_mismatch` with `warn` or `deny`) and `mismatch_allowed` (`agent_mismatch` that went through).
- **Guards:** denials per reason (`reread`, `generated_file`, `large_file`, `broad_search`) with bytes avoided and estimated tokens avoided (`bytes / 4`), most bytes first.

## Latency added by tokred

Hooks run synchronously before Claude Code continues, so `tokred stats latency` reports per hook: runs, p50, p95, p99 and max in ms; totals per session and per day; the average added per user prompt (`route`/`UserPromptSubmit` runs); counts of `ok`, `error`, `panic` and `timeout_guard`; and an alert when a hook's p95 passes the 50 ms budget (exactly 50 ms is within budget).

- Percentiles use the nearest-rank method: the p-th percentile of `n` sorted samples is the one at rank `ceil(p * n / 100)`. One sample is every percentile; ties return the tied value.
- Process spawn time is not visible from inside the binary, so `tokred doctor --bench` measures it from outside, and that value is added to **every** run, flagged `estimate: true`.
- A trade-off helper puts the total ms added next to the estimated tokens avoided by guards and the total saving (routing + guards) of the same period.

## With the proxy

When the optional [proxy](proxy.md) runs, it records its own, exact data per request in `proxy_requests`: requested and effective model, tokens (including 1-hour cache writes), time to first byte and total time, the likely cause of a prompt-cache miss, the origin (main loop, subagent, auxiliary, compaction), the agent type of a subagent and the time the proxy itself added (its overhead, budgeted at p95 under 10 ms). `stats`, `metrics` and the HTML report read the proxy as the **primary source** and the transcripts as the fallback for what the proxy did not see, as one merged list of usage rows.

### One source, no request counted twice

Every proxied request is also in the transcripts, so the two lists are merged before anything is computed (`merge_usage` in `tokred-metrics`):

- **Pairing.** A proxy request is paired with the transcript row that has the same `message_key` (the Anthropic message id and the request id, in the same format on both sides). A proxy request **without** a key (the response carried no message id, e.g. an interrupted stream) is paired with the transcript row of the same session whose four token counters are equal and whose time lies within 5 seconds of the request, or up to 5 seconds after the end of its response (the transcript line is written when the response ends). Two requests that both have a key and differ are two requests, never merged by time.
- **One to one.** A proxy request takes at most one transcript row and the reverse; keyless requests are paired nearest in time first. Two identical requests in the same few seconds stay two rows.
- **The proxy wins.** A pair becomes one row with the proxy's numbers: the model that really answered (not the one asked for), the tokens with the 1-hour cache split, the arrival time, the origin and the agent type. It keeps the transcript's project.
- **What stays.** Transcript-only rows (requests made while the proxy was off) and proxy-only rows (the project of its session when a transcript names it, else `(proxy)`).
- **What is skipped.** A request answered with a non-2xx status or with no tokens is not a message; it still counts for the overhead and the failed-request count.

Everything else reads the merged rows, so the totals, the groups, the all-on-opus counterfactual (the tokens are priced on the model that answered, so what routing really changed shows up in the routing saving), the projection and the before/after comparison are exact for the proxied part. Without any proxy row the transcript rows pass through unchanged and every output stays exactly as described above.

### What `stats` adds

- `--by origin` groups the cost by who sent the request (`main`, `subagent`, `auxiliary`, `compaction`) and `--by agent-type` by the agent type of subagent requests (`Explore`, `Plan`, ...). Without the proxy, `origin` falls back to the transcripts (`main` or `subagent`) and every agent type is `(unknown)`.
- A **Proxy** section after the cost tables, only when the proxy recorded something in the period:

| Line | How |
|---|---|
| Data source | Messages measured by the proxy and messages known only from the transcripts |
| Requests | Requests the proxy handled and how many the upstream did not answer with a 2xx status |
| Overhead | p50, p95, p99 and max of `proxy_overhead_us` in ms (nearest-rank, as for the hooks) against the 10 ms budget; an alert when the p95 is over it (exactly 10 ms is within budget) |
| Routing | Decisions of the period: applied (`enforce`), observed (`observe`), kept, fallback, with the **estimated** saving of the applied ones and of the observed ones had they been applied (`est_cost_from - est_cost_to` of each decision) |
| Model routing | The model asked for against the model that answered, per pair: messages, tokens and cost on the model that answered; rewritten pairs are marked |
| Cache misses | Main-loop messages whose `miss_cause` is set, and a table per cause with the tokens those requests wrote to the cache again |
| Suggestion | The latest suggestion not shown yet in the period, with a pointer to `tokred suggest`; `stats` never marks it as shown |

`stats --json` carries the same data under a top-level `proxy` key, present only when the proxy recorded something.

## Spans and ranges

`parse_span` accepts `<n>h`, `<n>d` or `<n>w` (n >= 1: `12h`, `7d`, `2w`, `30d`). A span turns into the half-open range `[now - span, now + 1)`, i.e. up to and including the current second. Feed `compute_stats` the rows from `earliest_usage()` to `now`, not only the range, or the projection and before/after will be wrong.

## HTML report

`tokred metrics` writes a self-contained, offline HTML file (inline CSS/JS, SVG charts generated in Rust, light/dark theme) to `<data>/reports/metrics.html`, always the same file, atomically, and opens it in the browser unless `--no-open`. Sections without data show a "no data yet" notice instead of disappearing. The proxy's sections (model routing, overhead, cache misses by cause, cost by origin and agent type) are the exception: they appear only when the proxy recorded something, so a report without the proxy is exactly the one it always was.

Prices come from the public LiteLLM JSON with an embedded fallback; an unknown model is mapped to the latest price of its family (opus/sonnet/haiku) and flagged.
