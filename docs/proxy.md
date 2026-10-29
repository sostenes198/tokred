**English** | [Português](proxy.pt-BR.md)

# The local proxy

The proxy is an optional local server between Claude Code and the Anthropic API: it measures every request exactly, can route subagent and auxiliary requests to a cheaper model, advises you about the main loop, and can keep large tool results out of the context without losing them.

```
Claude Code ──(ANTHROPIC_BASE_URL=http://127.0.0.1:1029)──► tokred proxy ──► api.anthropic.com
```

The rest of tokred (the [hooks and agents](hooks-and-agents.md), `stats`, `audit`) works with or without it.

## Commands

| Command | What it does |
|---|---|
| `tokred proxy enable` | Turns the proxy features on: chooses the port and content recording, saves them, starts the proxy and points Claude Code's settings at it |
| `tokred proxy disable` | Turns the features off and removes tokred's `ANTHROPIC_BASE_URL` from Claude Code's settings. A running proxy is not stopped |
| `tokred proxy start` | Starts the proxy in the background. `--foreground` keeps it in the terminal (the user service uses that), `--bind HOST:PORT` overrides `[proxy] bind` for this run. Refuses while the features are off |
| `tokred proxy stop` | Stops it and waits until it is gone |
| `tokred proxy status` | Is it running, on which port, and does this terminal use it (`--json` for scripts) |
| `tokred proxy service install` / `uninstall` | Installs the proxy as a user service that starts at login (systemd user unit, launchd agent, Windows logon task). Refuses while the features are off |
| `tokred claude [args]` | Starts the proxy if needed and runs `claude` through it, for this run only. Arguments and exit code pass through; nothing is written to Claude Code's settings. Without the proxy features it runs `claude` directly |
| `tokred dashboard` | Opens the [live dashboard](dashboard.md) |

The [install script](installation.md) runs `tokred proxy enable` (unless you pass `--no-proxy`), and installs the `claude` shell wrapper that runs `tokred initialize` before every launch. `tokred configure` only sets up hooks and agents and never touches the proxy.

### `tokred proxy enable`

| Flag | Meaning |
|---|---|
| `--port <n>` (or `TOKRED_PORT`) | Port of `[proxy] bind`, 1024 to 65535 on `127.0.0.1`. Asked in a terminal (default: the saved port, else 1029; the command checks that the port is free). A busy port given by flag only warns |
| `--record-content <true\|false>` (or `TOKRED_RECORD_CONTENT`) | `[proxy] log_bodies`: whether the [dashboard](dashboard.md) may show what was sent and returned. Asked in a terminal (default: the saved answer, else yes). Without a terminal an unsaved answer is no |
| `--yes`, `-y` | Ask nothing: use the flags, the saved values, else port 1029 and no recording |

A flag wins over its environment variable, and your other keys in `tokred.toml` stay. If the proxy is already running on another port, or with another recording setting, it is restarted to apply the change.

### The switch: `[proxy] enabled`

`enabled` in `tokred.toml` is the one switch of the proxy features. It is `false` until you run `tokred proxy enable`.

`tokred initialize` makes Claude Code's `settings.json` agree with it, and is what the `claude` wrapper runs before every launch. It reconciles one key, `env.ANTHROPIC_BASE_URL` (`http://127.0.0.1:<port>`, from `[proxy] bind`):

| `enabled` | The value is absent or tokred's | The value is another gateway |
|---|---|---|
| `true` | Starts the proxy if it is not running and, once it answers, sets the URL. If it cannot start, the URL is removed again (a URL that points at a dead proxy would stop Claude Code) and a warning is printed | Nothing is changed or started; one line on stderr says so |
| `false` | Removes tokred's URL. A running proxy is not stopped | Left alone |

"Tokred's URL" is the current proxy URL or any URL tokred wrote before (for example after a port change). Your own gateway is never overwritten, even one on `127.0.0.1`. `initialize` never fails and prints nothing unless there is a warning (`--verbose` says what it did, `--json` prints a report).

While the URL is in `settings.json`, Claude Code started from anywhere (an IDE, a terminal without the wrapper) goes through the proxy, and if the proxy is down it cannot reach the API. Three safeguards cover that: the user service, the `session-start` hook (it starts the proxy in the background when the session points at it and it does not answer), and `tokred doctor` (it warns when the URL is set and the proxy is down). To get out of any trouble run `tokred proxy disable` (or `tokred uninstall`, or delete `ANTHROPIC_BASE_URL` from the `env` object of `settings.json`).

## What it does

| Feature | Default | What it changes |
|---|---|---|
| Measurement | always on | Nothing. Records each request's model, tokens (including 1-hour cache writes), latency, stop reason, tool names, cache lifetime, the routing mode in force and the likely cause of a prompt-cache miss. `tokred stats`, `tokred metrics` and the report read them as their exact source ([metrics](metrics.md)) |
| [Model routing](routing.md) | `observe` | Only subagent and auxiliary requests, and only in `enforce`. **The main loop's model is never changed** |
| Suggestions | shown by `tokred suggest` and `tokred stats` | Nothing. Tells you when the main loop's last prompts were simple enough for a cheaper model |
| Reversible compression | `observe` | Only in `enforce`: large tool results are replaced by a preview and an id; the original is kept and the model reads it back with `tokred_retrieve`. Turn it on with `tokred compression on` |
| Response-cache measurement | on | Nothing. Counts exact repeats to show whether a response cache would help |

`observe` means the feature records what it would do, so `tokred routing status`, `tokred suggest` and `tokred stats` show the saving before you turn anything on. The keys are in [configuration](configuration.md).

## Guarantees

- **Fail-open.** If any step fails, the original request is forwarded unchanged. A rewritten request that the API rejects (400, 404 or 422) before anything reached Claude Code is sent again in its original form. A tokred bug never stops Claude Code from working.
- **Loopback only.** It refuses to listen on a non-loopback address, answers only requests whose `Host` header names a loopback address and its own port (the defence against DNS rebinding), and its admin endpoints (`/__tokred/...`) need a secret kept in `<data>/proxy/secret` with owner-only permissions.
- **Nothing it does not understand is changed.** Headers (`anthropic-beta`, `x-claude-code-*`, credentials) and fields pass through untouched, so new Claude Code features keep working. The only header it rewrites is `Accept-Encoding`, set to `identity` upstream so the response can be read as it streams.
- **The main loop's model is never changed**, whatever the configuration says.
- **Private.** Credentials are never logged or stored. Message contents are not read or stored unless you turn on `[proxy] log_bodies` (see [dashboard](dashboard.md)).

## Logins

API-key and claude.ai subscription logins both work. With a subscription the credential travels in the `authorization` header, and the proxy forwards it and the OAuth `anthropic-beta` value untouched. Claude Code's own documentation describes measuring subscription traffic through a local gateway.

Anthropic's terms do not address rewriting the model of subscription requests, so routing stays in `observe` by default for every login; `tokred doctor` says when `enforce` is on with a subscription login. Turning it on is your informed choice ([decisions](decisions.md)).

## Behind a corporate network

The proxy reaches the API the way Claude Code does.

| Setting | Effect |
|---|---|
| `HTTPS_PROXY` (then `ALL_PROXY`, `HTTP_PROXY`, and the lower-case names) | The proxy asks that HTTP proxy for a `CONNECT` tunnel to the API host. `user:password@` is sent as basic authentication and never logged |
| `NO_PROXY` | As in curl: comma-separated host names (a suffix also matches subdomains), IP addresses, CIDR ranges, `*` |
| `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`, `[proxy] ca_bundle` | PEM files of extra certificate authorities, trusted on top of the built-in roots and the operating system's store. This is what a TLS-inspecting proxy needs |

A value that is not a proxy address, or a CA file that is missing or empty, is logged once (by name, never the value) and ignored: the proxy connects directly, or with the remaining roots. A proxy that refuses the tunnel, or an untrusted certificate, gives the usual `502`. `tokred doctor` and `tokred proxy status` show the proxy (host and port only) and the extra CA files in use.

The settings are read once, when the proxy starts, from the environment of the process that starts it. Set the variables before `tokred proxy start` or `tokred claude` (for the user service, in the environment the service runs in, or use `ca_bundle`) and restart the proxy after changing them.

## Retention

What the proxy stores is deleted after `[proxy] retention_days` (default 90; `0` keeps everything), once when the proxy starts and then every hour. Kept tool results and replacements of a conversation that still has a request inside the period are kept whatever their age.

## Limits

- Only `http://` proxies are supported (no SOCKS, no TLS to the proxy).
- Usage is read only from an uncompressed response. If a response comes back compressed anyway it is forwarded unchanged and not measured.
- A compaction request carries no header that tells it apart, so it can be taken for a simple prompt. Routing never changes the main loop's model, so this only affects the advice.
