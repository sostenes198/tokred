**English** | [Português](features.pt-BR.md)

# Turning features on and off

Every optional part of tokred is switched with one command or one line of `tokred.toml`. This page lists them, what each one changes and when it takes effect. The commands are in [commands](commands.md), the keys in [configuration](configuration.md).

## At a glance

| Feature | Turn on | Turn off | Takes effect |
|---|---|---|---|
| Hooks and agents | `tokred configure` | `tokred unconfigure` | After you restart Claude Code |
| One hook | `[modes]` key set to `warn` or `enforce` | the key set to `"off"` | The next time that hook runs |
| Model-switch guard | `tokred model-guard on` (asks) or `warn` (says what it costs) | `tokred model-guard off` | The next time the hook runs |
| Proxy features | `tokred proxy enable` | `tokred proxy disable` | At once |
| The proxy process | `tokred proxy start` | `tokred proxy stop` | At once; `tokred proxy restart` restarts it |
| Start at login | `tokred proxy service install` | `tokred proxy service uninstall` | At the next login |
| Conversation text in the dashboard | `tokred proxy enable --record-content true` | `tokred proxy enable --record-content false` | At once; a running proxy is restarted |
| Model routing | `tokred routing on` | `tokred routing off` | At once; a running proxy is restarted |
| Reversible compression | `tokred compression on` | `tokred compression off` | After `tokred proxy restart` and restarting Claude Code |
| Suggestions | `[suggest] channel` set to `stats`, `statusline` or `message` | `channel = "off"` | After `tokred proxy restart` |
| Update notices | `[update] mode = "notify"` | `mode = "off"`, or `TOKRED_NO_UPDATE_CHECK=1` | The next command |
| Automatic network calls | unset `TOKRED_OFFLINE` | `TOKRED_OFFLINE=1` | For the process that has it set |
| The `claude` wrapper | the installer's default | install with `--no-alias`; `command claude` skips it for one run | The next shell |

`tokred uninstall` removes all of it. The main loop's model is never changed by any switch here.

## Hooks and agents

`tokred configure` adds tokred's hooks and its six agents to Claude Code's settings and agents folder; `tokred unconfigure` takes them out again (`--purge` also deletes tokred's data folder). Neither touches the proxy. To silence one hook without removing anything, set its mode to `"off"` in `tokred.toml`:

```toml
[modes]
guard_read = "off"
```

What each hook does, and what `warn` and `enforce` mean, is in [hooks and agents](hooks-and-agents.md).

## The proxy

`tokred proxy enable` is the master switch: it saves the port and whether conversation text is recorded, starts the proxy and points Claude Code at it. `tokred proxy disable` turns that off and removes tokred's address from Claude Code's settings; a running proxy keeps running until `tokred proxy stop`. The features below only matter while the proxy runs. Measurement is always on; the others have their own switch:

- **Routing** moves subagent and auxiliary requests to a cheaper model when it is safe. It starts in `observe` (records what it would do, changes nothing). See [routing](routing.md).
- **Compression** replaces large tool results with a preview the model can expand with `tokred_retrieve`. It starts in `observe` (measures only). See [proxy](proxy.md).
- **Suggestions** advise a cheaper model for the main loop; they never switch it.
- **Conversation text** is what the [dashboard](dashboard.md) shows of each request and response. It stays in tokred's database on this machine.

## Routing: check before you turn it on

Routing does not turn itself on and does not learn: it stays in `observe` until you run `tokred routing on`. Ask what it would have done over a period first:

```sh
tokred routing status --since 14d
```

The answer says how many requests routing would have changed, how much that would have saved and what that comes to in a month. When none would change, turning it on changes nothing yet. `tokred doctor` and `tokred compression status` give the same kind of answer for routing and for compression while they are in `observe`, from the last 7 days of measurements (all of them estimates). When one of them is clearly worth it (routing: more than about $1 in 7 days; compression: more than 5% of the input tokens), tokred also says so once a week, in a single line in your chat that only you see; `[suggest] savings_hint = false` turns that line off.

## Applying a change to the proxy

The proxy reads its settings when it starts. `tokred routing on` and `off` restart a running proxy for you; after editing `tokred.toml` yourself, or after `tokred compression on`, restart it:

```sh
tokred proxy restart
```

`restart` stops the proxy and starts it again (a proxy that was not running is just started). It refuses while the proxy features are off.
