**English** | [Português](metrics.pt-BR.md)

# Metrics

> **Planned (phase F1 onward).** This page documents the design so the numbers mean what they say once they exist. See the [spec](spec.md), section 10, for the full detail.

## Ingestion

tokred reads `<claude>/projects/**/*.jsonl` incrementally (the `ingest_state` table keeps each file's offset and mtime) and extracts, from assistant messages: model, `input_tokens`, `output_tokens`, `cache_creation_input_tokens` and `cache_read_input_tokens`. Lines are deduplicated by message id + request id because streaming can repeat them, and subagent lines are flagged. Field names are confirmed against a real transcript before the ingester is written.

## What `tokred stats` computes

| Number | How |
|---|---|
| Real cost | Sum over models of tokens × price |
| Counterfactual "everything on opus" | The same tokens at the opus price; the difference is the **routing saving** |
| Guard saving (**estimate**) | `bytes_saved / 4` as avoided tokens × the session model's input price |
| Prompt cache rate | `cache_read / (cache_read + cache_write + input)` |
| Projection | Daily average of the last 7 and 30 days × 30 |
| Before/after | Cost per session before vs. after the first `configure`, when older transcripts exist |

Anything derived from bytes (`bytes / 4`, characters / 4 in the audit) is always labelled an estimate.

## Latency added by tokred

Hooks run synchronously before Claude Code continues, so `tokred stats latency` reports per hook: runs, p50, p95, p99 and max in ms; totals per session and per day; counts of `error`, `panic` and `timeout_guard`; and an alert when a hook's p95 passes the 50 ms budget. Process spawn time is not visible from inside the binary, so `tokred doctor --bench` measures it from outside and adds it to the internal durations, labelled as an estimate.

## HTML report

`tokred metrics` writes a self-contained, offline HTML file (inline CSS/JS, SVG charts generated in Rust, light/dark theme) to `<data>/reports/metrics.html`, always the same file, atomically, and opens it in the browser unless `--no-open`. Sections without data show a "no data yet" notice instead of disappearing.

Prices come from the public LiteLLM JSON with an embedded fallback; an unknown model is mapped to the latest price of its family (opus/sonnet/haiku) and flagged.
