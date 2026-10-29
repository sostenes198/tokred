**English** | [Português](dashboard.pt-BR.md)

# The live dashboard

The [local proxy](proxy.md) serves two pages on its own port, in your browser:

- **`/dashboard`**: every session that is active right now and the last 10 that went idle or ended. Click one to see what is happening: each request Claude Code sent, what the model answered (streamed as it arrives), the tools it called, the cost, the tokens, the model routing decision, and the subagents running. Updates are pushed in real time (server-sent events).
- **`/metrics`**: the same self-contained HTML report that `tokred metrics` writes (last 30 days, without the project audit).

Both work offline: the pages are embedded in the binary and load nothing from the network.

## Opening it

```sh
tokred dashboard            # opens the browser
tokred dashboard --no-open  # prints the address instead
```

The proxy must be running (`tokred proxy start` or `tokred claude`); `tokred dashboard` starts nothing and says so if the proxy is down. Other users of the machine can reach `127.0.0.1`, so the pages ask you to sign in: the command gets a **one-time token** from the proxy (using the admin secret in `<data>/proxy/secret`) and opens `http://127.0.0.1:<port>/dashboard?token=...`. The proxy trades the token for a cookie (`HttpOnly`, `SameSite=Strict`) and redirects to `/dashboard`. The token in the address **works once and expires after 60 seconds**; run `tokred dashboard` again for a new one. Cookies live in the proxy's memory: restarting the proxy signs you out. Without a valid cookie every dashboard page answers `401` with a note to run `tokred dashboard`.

Browsers on other sites cannot use the dashboard: requests marked `Sec-Fetch-Site: cross-site` or `same-site` are refused (`403`), no CORS headers are sent, and the pages carry a strict `Content-Security-Policy` (the app's text is always rendered as text, never as HTML, because tool results and model output can contain hostile markup).

## Privacy: `[proxy] log_bodies`

What the dashboard can show depends on one setting, off by default:

| `log_bodies` | Shown and stored |
|---|---|
| `false` (default) | **Metadata only**: models, tokens, cost, timings, HTTP status, stop reason, names of the tools called, routing decision, subagents. Nothing about message contents is read from requests or responses, held in memory, sent to the browser or stored. The pages say that contents are not recorded. |
| `true` | Also the **contents**: for each request the messages that are new since the previous request of the same conversation (not the whole history that is re-sent every turn), and the response (text, reasoning, tool calls with a short summary of their input). Texts are cut at 20,000 characters (`…[truncated N chars]`). Headers and credentials are never captured. |

With `log_bodies = true`, contents are stored in the database (table `captured_turns`) and are kept only for sessions that are active or among the last 10, and never longer than `[proxy] retention_days` (default 90). Pruning runs when the proxy starts and every hour; it empties the contents (the metadata row stays until `retention_days` passes). Everything stays on your machine. See the [decision](decisions.md) for the trade-off.

## What it shows

- A session is **active** while a request is in flight or its last one was less than 5 minutes ago, **idle** for the next 55 minutes and **ended** after that.
- A subagent counts as **running** while a request of it is in flight and for 60 seconds after its last one.
- Costs use the price table of `tokred pricing`; a model without a price shows no cost.
- A session detail has the newest 500 turns; `"truncated": true` says older ones exist in the database.
- Requests without a session id (some auxiliary calls) are not followed.

## API reference

All are `GET`, answered by the proxy and never forwarded to the Anthropic API. They need the session cookie.

| Path | Answer |
|---|---|
| `/metrics` | The HTML report |
| `/dashboard`, `/dashboard/app.css`, `/dashboard/app.js` | The single-page app |
| `/dashboard/api/sessions` | `{"active": [S], "recent": [S]}`; `S.status` is `active` for the first list and `idle` or `ended` for the second |
| `/dashboard/api/sessions/{id}` | `{"session": S, "turns": [T], "truncated"?: true}`, oldest turn first; a turn in flight has `in_flight: true` and the text received so far in `response.text` |
| `/dashboard/events` (`?session=<id>`) | `text/event-stream`: `session`, `turn_start`, `turn_delta`, `turn_end`, `subagent`; a `: ping` comment every 15 seconds. Events are not replayed after a reconnect: read the API again |
| `/favicon.ico`, `/dashboard/favicon.ico` | `204` |

`S` (session): `id`, `started_at`, `last_activity_at` (unix seconds), `status`, `requests`, `models`, `cost_usd`, `input`, `output`, `cache_read`, `cache_write`, `subagents_running`, `content_recorded`.

`T` (turn, one request and its response): `id`, `session_id`, `ts` (unix seconds), `origin` (`main`, `subagent` or `auxiliary`; a compaction request is `auxiliary`), `agent_id`, `agent_type`, `model_requested`, `model_effective`, `status`, `stop_reason`, `usage` (`input`, `output`, `cache_write`, `cache_read`), `cost_usd`, `duration_ms`, `ttfb_ms`, `tools`, `routing` (`outcome`, `reason`), `in_flight`, `sent` (`[{role, text}]` or `null`), `response` (`{text, thinking, tool_uses: [{name, input_summary}]}` or `null`).

Events: `session` carries an `S`; `turn_start` a `T` with `in_flight: true` and no response; `turn_delta` `{session_id, turn_id, text}` to append to the response; `turn_end` the final `T`; `subagent` `{session_id, agent_id, agent_type, state: "running" | "finished", ts}`.

Other methods on these paths answer `405`; a wrong `Host` answers `421`.
