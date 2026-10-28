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
| Suggestions | shown in `stats` | Nothing. Tells you when the main loop's last prompts were simple enough for a cheaper model |
| Reversible compression | `observe` | Only in `enforce`: large tool results are replaced by a preview plus an id; the original is kept and the model reads it back with `tokred_retrieve` |
| Response-cache measurement | on | Nothing. Counts exact repeats to show whether a response cache would help |

`observe` means the feature records what it *would* do, so `tokred stats` and `tokred metrics` show the saving before you turn anything on.

## Guarantees

- **Fail-open.** If any step fails, the original request is forwarded unchanged. A rewritten request that the API rejects before anything reached Claude Code is sent again in its original form. The proxy never makes Claude Code stop working because of a tokred bug.
- **Loopback only.** It listens on `127.0.0.1`, checks the `Host` header, and its admin endpoints need a secret stored in `<data>/proxy/secret` (owner-only permissions).
- **Forwards everything it does not understand.** Headers (`anthropic-beta`, `x-claude-code-*`, credentials) and fields are passed through unchanged, so new Claude Code features keep working.
- **Private.** Credentials and message contents are never logged or stored; only counts, hashes and the structure of requests.

## Logins

Both API-key and claude.ai subscription logins work (verified in the F7 spike). With a subscription, the credential travels in the `authorization` header and the proxy forwards it and the OAuth `anthropic-beta` value untouched. Measuring subscription traffic through a local gateway is a setup the Claude Code documentation describes. Rewriting the model of subscription requests is not addressed by Anthropic's terms, so routing stays in `observe` unless you turn it on ([decision](decisions.md)).

## Configuration

`[proxy]`, `[routing]`, `[suggest]`, `[compression]` and `[shadow]` in `tokred.toml`; see [configuration](configuration.md).
