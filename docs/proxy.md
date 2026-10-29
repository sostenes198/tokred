**English** | [Português](proxy.pt-BR.md)

# The local proxy

`tokred proxy` is an optional local server between Claude Code and the Anthropic API. It sees every request and response, so it can measure them exactly, route subagent and auxiliary requests to a cheaper model, advise you about the main loop, and keep large tool results out of the context without losing them.

```
Claude Code ──(ANTHROPIC_BASE_URL=http://127.0.0.1:8787)──► tokred proxy ──► api.anthropic.com
```

Everything else in tokred (the subagents, the guard hooks, `stats`, `audit`) works with or without the proxy.

## Using it

```sh
tokred claude            # start the proxy if needed and open Claude Code through it, for this run only
tokred proxy status      # is it running, and does this terminal use it?
tokred proxy stop
```

`tokred claude` passes every argument to `claude` unchanged and exits with `claude`'s exit code. Nothing is written to Claude Code's settings. If the proxy cannot start, or `ANTHROPIC_BASE_URL` already points to another gateway, Claude Code runs exactly as before and tokred says why on stderr.

To have the proxy always available, install it as a user service that starts at login (a systemd user unit on Linux, a launchd agent on macOS, a logon task on Windows):

```sh
tokred proxy service install      # and later: tokred proxy service uninstall
```

`tokred proxy start` runs it in the background without a service; `--foreground` keeps it in the terminal (the service uses that).

## What it does

| Feature | Default | What it changes |
|---|---|---|
| Measurement | always on | Nothing. Records each request's model, tokens (including 1-hour cache writes), latency and the likely cause of a prompt-cache miss |
| [Model routing](routing.md) | `observe` | Only subagent and auxiliary requests, and only in `enforce`. **The main loop's model is never changed** |
| Suggestions | shown by `tokred suggest` | Nothing. Tells you when the main loop's last prompts were simple enough for a cheaper model |
| Reversible compression | `observe` | Only in `enforce`: large tool results are replaced by a preview plus an id; the original is kept and the model reads it back with `tokred_retrieve` |
| Response-cache measurement | on | Nothing. Counts exact repeats to show whether a response cache would help |

`observe` means the feature records what it *would* do, so `tokred routing status` and `tokred suggest` show the saving before you turn anything on.

## Guarantees

- **Fail-open.** If any step fails, the original request is forwarded unchanged. A rewritten request that the API rejects before anything reached Claude Code is sent again in its original form. The proxy never makes Claude Code stop working because of a tokred bug.
- **Loopback only.** It listens on `127.0.0.1`, checks the `Host` header, and its admin endpoints need a secret stored in `<data>/proxy/secret` (owner-only permissions).
- **Forwards everything it does not understand.** Headers (`anthropic-beta`, `x-claude-code-*`, credentials) and fields are passed through unchanged, so new Claude Code features keep working.
- **Private.** Credentials are never logged or stored. Message contents are not read, kept or stored unless you turn on `[proxy] log_bodies` (off by default), which feeds the [dashboard](dashboard.md); without it only counts, hashes and the structure of requests are recorded. See the [decision](decisions.md).

## Dashboard and report

The proxy also answers `GET /dashboard` (live sessions, what was sent and answered, subagents; server-sent events) and `GET /metrics` (the HTML report of `tokred metrics`) on its own port. They are never forwarded upstream, need a sign-in (`tokred dashboard` opens it) and carry no CORS headers. See [dashboard](dashboard.md).

## Logins

Both API-key and claude.ai subscription logins work (verified in the F7 spike). With a subscription, the credential travels in the `authorization` header and the proxy forwards it and the OAuth `anthropic-beta` value untouched. Measuring subscription traffic through a local gateway is a setup the Claude Code documentation describes. Rewriting the model of subscription requests is not addressed by Anthropic's terms, so routing stays in `observe` unless you turn it on ([decision](decisions.md)).

## Limits

- The proxy connects to the API directly and trusts the built-in certificate roots. It does not read `HTTPS_PROXY` or a custom CA bundle, so it does not work behind a corporate proxy that inspects TLS. In that case use Claude Code without it.
- It reads response usage only from an uncompressed response. It asks the API for one (`Accept-Encoding: identity`); if a response comes back compressed anyway it is still forwarded unchanged, just not measured.
- A compaction request carries no header that tells it apart, so it can be taken for a simple prompt. Routing never changes the main loop's model, so this only affects the advice.
- On Windows, `tokred proxy start` run with its output captured by a pipe (for example `out=$(tokred proxy start)`, or a CI step that records output) does not return until the proxy stops: Windows lets the background process inherit the pipe. Run it in a terminal, send its output to a file or `NUL`, or use `tokred proxy service install`. The same applies to `tokred claude` when it has to start the proxy with its output piped; start the proxy first in that case.
- Per-request records do not include the cache lifetime used. The stop reason and the tool names are kept in `captured_turns` (see [dashboard](dashboard.md)).

## Configuration

`[proxy]`, `[routing]`, `[suggest]`, `[compression]` and `[shadow]` in `tokred.toml`; see [configuration](configuration.md).
