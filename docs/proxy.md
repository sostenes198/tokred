**English** | [Português](proxy.pt-BR.md)

# The local proxy

`tokred proxy` is an optional local server between Claude Code and the Anthropic API. It sees every request and response, so it can measure them exactly, route subagent and auxiliary requests to a cheaper model, advise you about the main loop, and keep large tool results out of the context without losing them.

```
Claude Code ──(ANTHROPIC_BASE_URL=http://127.0.0.1:1029)──► tokred proxy ──► api.anthropic.com
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
- **Private.** Credentials and message contents are never logged or stored; only counts, hashes and the structure of requests.

## Logins

Both API-key and claude.ai subscription logins work (verified in the F7 spike). With a subscription, the credential travels in the `authorization` header and the proxy forwards it and the OAuth `anthropic-beta` value untouched. Measuring subscription traffic through a local gateway is a setup the Claude Code documentation describes. Rewriting the model of subscription requests is not addressed by Anthropic's terms, so routing stays in `observe` unless you turn it on ([decision](decisions.md)).

## Behind a corporate proxy

The proxy reaches the API the way Claude Code does when your network requires it.

- **HTTP proxy.** If `HTTPS_PROXY` is set (also `ALL_PROXY` and `HTTP_PROXY`, in that order, and the lower-case names), the proxy asks it for a tunnel with `CONNECT` to the API host. `user:password@` in the address is sent as basic authentication and never logged. `NO_PROXY` works as in curl: comma-separated host names (a suffix also matches subdomains), IP addresses, CIDR ranges, `*`. SOCKS (`socks5://`) and `https://` proxies are not supported; tokred says so in the log and in `tokred doctor`, and connects directly.
- **Certificate authorities.** On top of the built-in roots it trusts your operating system's certificate store and the PEM files named by `NODE_EXTRA_CA_CERTS` and `SSL_CERT_FILE` (the variables Claude Code reads) and by `ca_bundle` in `[proxy]` of `tokred.toml`. This is what a TLS-inspecting proxy needs.
- **Fail-open.** A value that is not a proxy address, or a CA file that is missing or has no certificate, is logged once (by variable or file name, never the value) and ignored: the proxy connects directly, or with the remaining roots. A proxy that is reachable but refuses the tunnel (or a certificate that is not trusted) gives the usual `502` for an upstream that cannot be reached.
- `tokred doctor` and `tokred proxy status` show the proxy (host and port only) and the extra CA files in use.

The settings are read when the proxy starts, from the environment of the process that starts it: set the variables before `tokred proxy start` or `tokred claude`, and for the user service in the environment the service runs in (or put the CA file in `ca_bundle`). Restart the proxy after changing them.

## Limits

- Behind a corporate network only `http://` proxies are supported (no SOCKS), and the proxy settings are read once, when the proxy starts.
- It reads response usage only from an uncompressed response. It asks the API for one (`Accept-Encoding: identity`); if a response comes back compressed anyway it is still forwarded unchanged, just not measured.
- A compaction request carries no header that tells it apart, so it can be taken for a simple prompt. Routing never changes the main loop's model, so this only affects the advice.
- Per-request records do not yet include the stop reason, the tool names or the cache lifetime used.

## Configuration

`[proxy]`, `[routing]`, `[suggest]`, `[compression]` and `[shadow]` in `tokred.toml`; see [configuration](configuration.md).
