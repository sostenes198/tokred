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

The proxy features are off until `tokred proxy enable` has been run (the install script runs it, unless you pass `--no-proxy`; see [the switch](#the-switch-proxy-enabled)): while they are off `tokred claude` runs `claude` directly.

`tokred claude` passes every argument to `claude` unchanged and exits with `claude`'s exit code. Nothing is written to Claude Code's settings. If the proxy cannot start, or `ANTHROPIC_BASE_URL` already points to another gateway, Claude Code runs exactly as before and tokred says why on stderr.

### The switch: `[proxy] enabled`

`enabled` in `tokred.toml` is the one switch of the proxy features. It is `false` until you run `tokred proxy enable` (the install script does, unless you pass `--no-proxy`); `tokred configure` only sets up the hooks and agents and never touches the proxy. `tokred proxy enable` and `tokred proxy disable` change it and take effect at once.

```sh
tokred proxy disable     # features off: tokred's URL is removed from Claude Code's settings, nothing else changes
tokred proxy enable      # features on: choose the port and content recording, start the proxy, point Claude Code's settings at it
tokred initialize        # what the `claude` shell wrapper runs before each launch (see below)
```

`tokred proxy enable` is where the proxy is set up. It chooses two things and saves them in `tokred.toml` together with `enabled = true`:

```sh
tokred proxy enable                                    # in a terminal: asks the port and whether to record content
tokred proxy enable --port 1050 --record-content true  # answers up front, nothing is asked
tokred proxy enable --yes                              # asks nothing: saved values, else port 1029 and no recording
```

- **`--port <n>`** (or `TOKRED_PORT`) is the port of `[proxy] bind` (1024 to 65535, on `127.0.0.1` unless your `bind` names another host). Asked, the default is the saved port, else 1029; the command binds the port to see that it is free, asks again when it is busy, and accepts the port the proxy of this installation already listens on. Given by flag, a busy port only warns.
- **`--record-content <true|false>`** (or `TOKRED_RECORD_CONTENT`; `yes`, `no`, `1` and `0` also work) is `[proxy] log_bodies`: whether the [dashboard](dashboard.md) may show what was sent and returned. The text stays in tokred's own database on this machine; headers and credentials are never recorded. Asked, the default is the saved answer, else yes; without a terminal, an unsaved answer is no.

A flag wins over its environment variable. Your other keys in `tokred.toml` stay. If the proxy is already running on another port than the new one, it is stopped and started again on the new port. Then Claude Code's settings are reconciled, as described next.

`tokred initialize` makes Claude Code's `settings.json` agree with the switch, and it is what the `claude` wrapper installed by the installer runs before every launch. The one thing it reconciles is `env.ANTHROPIC_BASE_URL` (`http://127.0.0.1:<port>` from `[proxy] bind`):

| `enabled` | The value is absent or tokred's | The value is another gateway |
|---|---|---|
| `true` | Starts the proxy if it is not running and, once it answers, sets the URL. If the proxy cannot be started the URL is removed again (a URL that points at a dead proxy would stop Claude Code) and a warning is printed | Nothing is changed and nothing is started; one line on stderr says tokred is enabled but `ANTHROPIC_BASE_URL` points elsewhere |
| `false` | Removes tokred's URL (and the `env` object if that empties it). A running proxy is not stopped | Left alone |

"Tokred's URL" is the current proxy URL or any URL tokred wrote before (for example after you changed the port); a value exported in the shell counts as the current value too. Your own gateway is never overwritten, even one on `127.0.0.1`. `initialize` never fails and never prints anything unless there is a warning (`--verbose` says what it did, `--json` prints a report); when everything is already right it costs one file read and one health probe.

While the URL is in `settings.json`, Claude Code started from anywhere (an IDE, a terminal without the wrapper) goes through the proxy. If the proxy is not running at that moment the API cannot be reached, so there are three safeguards: the user service below; the `session-start` hook, which starts the proxy in the background when it is enabled, the session is pointed at it and it does not answer (see [hooks](hooks.md)); and `tokred doctor`, which warns when the URL is set and the proxy is down. To get out of any trouble: `tokred proxy disable` (or `tokred unconfigure`, or delete `ANTHROPIC_BASE_URL` from the `env` object of `settings.json`).

To have the proxy always available, install it as a user service that starts at login (a systemd user unit on Linux, a launchd agent on macOS, a logon task on Windows):

```sh
tokred proxy service install      # and later: tokred proxy service uninstall
```

`tokred proxy start` runs it in the background without a service; `--foreground` keeps it in the terminal (the service uses that). Both refuse while `enabled = false`.

With `enabled = false`, `tokred claude` runs `claude` directly and says so on stderr.

## What it does

| Feature | Default | What it changes |
|---|---|---|
| Measurement | always on | Nothing. Records each request's model, tokens (including 1-hour cache writes), latency, the stop reason, the names of the tools offered, the cache lifetime asked for (`5m` or `1h`), the routing mode in force and the likely cause of a prompt-cache miss; `tokred stats`, `tokred metrics` and the HTML report read them as their exact source ([metrics](metrics.md)) |
| [Model routing](routing.md) | `observe` | Only subagent and auxiliary requests, and only in `enforce`. **The main loop's model is never changed** |
| Suggestions | shown by `tokred suggest` and `tokred stats` (and, if you choose, the status line and a one-line message) | Nothing. Tells you when the main loop's last prompts were simple enough for a cheaper model |
| Reversible compression | `observe` | Only in `enforce`: large tool results are replaced by a preview plus an id; the original is kept and the model reads it back with `tokred_retrieve` |
| Response-cache measurement | on | Nothing. Counts exact repeats to show whether a response cache would help |

`observe` means the feature records what it *would* do, so `tokred routing status`, `tokred suggest` and `tokred stats` show the saving before you turn anything on.

## Guarantees

- **Fail-open.** If any step fails, the original request is forwarded unchanged. A rewritten request that the API rejects before anything reached Claude Code is sent again in its original form. The proxy never makes Claude Code stop working because of a tokred bug.
- **Loopback only.** It listens on `127.0.0.1`, checks the `Host` header, and its admin endpoints need a secret stored in `<data>/proxy/secret` (owner-only permissions).
- **Forwards everything it does not understand.** Headers (`anthropic-beta`, `x-claude-code-*`, credentials) and fields are passed through unchanged, so new Claude Code features keep working.
- **Private.** Credentials are never logged or stored. Message contents are not read, kept or stored unless you turn on `[proxy] log_bodies` (off by default), which feeds the [dashboard](dashboard.md); without it only counts, hashes and the structure of requests are recorded. See the [decision](decisions.md).

## Dashboard and report

The proxy also answers `GET /dashboard` (live sessions, what was sent and answered, subagents; server-sent events) and `GET /metrics` (the HTML report of `tokred metrics`) on its own port. They are never forwarded upstream, need a sign-in (`tokred dashboard` opens it) and carry no CORS headers. See [dashboard](dashboard.md).

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

## Retention

What the proxy stores is deleted after `[proxy] retention_days` (default 90; `0` keeps everything): once when the proxy starts and then every hour. See [configuration](configuration.md).

## Configuration

`[proxy]`, `[routing]`, `[suggest]`, `[compression]` and `[shadow]` in `tokred.toml`; see [configuration](configuration.md).
