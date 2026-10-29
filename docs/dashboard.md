**English** | [Português](dashboard.pt-BR.md)

# The live dashboard

The [local proxy](proxy.md) serves two pages on its own port, to show in your browser what is happening in your Claude Code sessions right now. Both are embedded in the binary and load nothing from the network.

- **`/dashboard`**: every active session and the last 10 that went idle or ended. Open one to see each request Claude Code sent, what the model answered (streamed as it arrives), the tools it called, the cost, the tokens, the routing decision and the subagents running. It updates live (server-sent events).
- **`/metrics`**: the same HTML report that `tokred metrics` writes (last 30 days, without the project audit).

## Opening it

```sh
tokred dashboard
```

`tokred dashboard --no-open` prints the address instead of opening the browser.

The proxy must be enabled and running: `tokred proxy enable` starts it, and later `tokred claude`, the `claude` wrapper or `tokred proxy start` start it again. `tokred dashboard` starts nothing and says so if the proxy is down.

Other users of the machine can reach `127.0.0.1`, so the pages ask you to sign in. The command gets a **one-time token** from the proxy (using the admin secret in `<data>/proxy/secret`) and opens `http://127.0.0.1:<port>/dashboard?token=...`. The proxy trades the token for a cookie (`HttpOnly`, `SameSite=Strict`, valid for 12 hours) and redirects to `/dashboard`. The token works once and expires after 60 seconds; run `tokred dashboard` again for a new one. Cookies live in the proxy's memory, so restarting the proxy signs you out. Without a valid cookie every page answers `401` with a note to run `tokred dashboard`.

Browsers on other sites cannot use it: requests marked `Sec-Fetch-Site: cross-site` or `same-site` are refused (`403`), no CORS headers are sent, and the pages carry a strict `Content-Security-Policy`. Text from requests and responses is always rendered as text, never as HTML, because tool results and model output can contain hostile markup.

## Privacy: `[proxy] log_bodies`

What the dashboard can show depends on one setting, off by default. `tokred proxy enable` asks about it.

| `log_bodies` | Shown and stored |
|---|---|
| `false` (default) | **Metadata only**: models, tokens, cost, timings, HTTP status, stop reason, names of the tools called, routing decision, subagents. Nothing about message contents is read, held in memory, sent to the browser or stored; the pages say so |
| `true` | Also the **contents**: for each request the messages that are new since the previous request of the same conversation (not the whole history that is re-sent every turn), and the response (text, reasoning, tool calls with a short summary of their input). Each text is cut at 20,000 characters (`…[truncated N chars]`). Headers and credentials are never captured |

With `log_bodies = true`, contents are stored in the local database (table `captured_turns`) and kept only for sessions that are active or among the last 10, and never longer than `[proxy] retention_days` (default 90). Pruning runs when the proxy starts and every hour; it empties the contents and the metadata row stays until `retention_days` passes. Everything stays on your machine, but anyone who can read the database can read the contents. Changing the setting with `tokred proxy enable --record-content` restarts a running proxy so it takes effect. The trade-off is in [decisions](decisions.md).

## What it shows

- A session is named the way Claude Code names it: the title you gave it, else the one Claude Code generated, with the folder it runs in and a short id. The title is read from the end of the session's transcript under `projects/` (from its start when the end has none) and re-read while the session runs, because Claude Code only names a session after its first turn. A session without a transcript shows its short id. Only transcripts under the Claude Code folder are read, and only the title and the folder are taken from them.
- A session is **active** while a request is in flight or its last one was within the last 5 minutes, **idle** until an hour after its last activity, and **ended** after that.
- A subagent counts as **running** while a request of it is in flight and for 60 seconds after its last one.
- Costs use the price table of `tokred pricing`; a model without a price shows no cost.
- A session detail has the newest 500 turns; `"truncated": true` says older ones exist in the database.
- Requests without a session id (some auxiliary calls) are not followed.
- A compaction request is shown as `auxiliary`.

## API reference

For people who script it. All routes are `GET`, answered by the proxy and never forwarded to the Anthropic API, and need the session cookie. Other methods answer `405`, an unknown path under `/dashboard/` answers `404`, and a wrong `Host` answers `421`.

| Path | Answer |
|---|---|
| `/metrics` | The HTML report |
| `/dashboard`, `/dashboard/app.css`, `/dashboard/app.js` | The single-page app |
| `/dashboard/api/sessions` | `{"active": [S], "recent": [S]}`; `S.status` is `active` in the first list and `idle` or `ended` in the second |
| `/dashboard/api/sessions/{id}` | `{"session": S, "turns": [T], "truncated"?: true}`, oldest turn first; a turn in flight has `in_flight: true` and the text received so far in `response.text`. An unknown session answers `404` |
| `/dashboard/events` (`?session=<id>`) | `text/event-stream` with the events below and a `: ping` comment every 15 seconds. Events are not replayed after a reconnect: read the API again |
| `/favicon.ico`, `/dashboard/favicon.ico` | `204` |

Session `S`: `id`, `started_at`, `last_activity_at` (unix seconds), `status`, `requests`, `models`, `cost_usd`, `input`, `output`, `cache_read`, `cache_write`, `subagents_running`, `content_recorded`, `title` and `project` (the folder name; both `null` when the transcript is not found).

Turn `T` (one request and its response): `id`, `session_id`, `ts` (unix seconds), `origin` (`main`, `subagent` or `auxiliary`), `agent_id`, `agent_type`, `model_requested`, `model_effective`, `status`, `stop_reason`, `usage` (`input`, `output`, `cache_write`, `cache_read`), `cost_usd`, `duration_ms`, `ttfb_ms`, `tools`, `routing` (`outcome`, `reason`), `in_flight`, `sent` (`[{role, text}]` or `null`), `response` (`{text, thinking, tool_uses: [{name, input_summary}]}` or `null`).

Events:

| Event | Data |
|---|---|
| `session` | an `S` |
| `turn_start` | a `T` with `in_flight: true` and no response |
| `turn_delta` | `{session_id, turn_id, text}`, to append to the response (sent only with `log_bodies = true`) |
| `turn_end` | the final `T` |
| `subagent` | `{session_id, agent_id, agent_type, state: "running" \| "finished", ts}` |
