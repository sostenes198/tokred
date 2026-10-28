**English** | [Português](hooks.pt-BR.md)

# Hooks

`tokred configure` registers six hooks in `settings.json`. Each one runs `"<path>/tokred" hook <name>` with a 5 second timeout (Claude Code's `timeout` is in **seconds**).

| Hook | Claude Code event | Matcher | Purpose | Phase |
|---|---|---|---|---|
| `route` | `UserPromptSubmit` | – | Classify the prompt and suggest the right subagent (one line, only when the class changed) | F4 |
| `enforce` | `PreToolUse` | `Agent\|Task` | Compare the chosen subagent with the last classification | F4 |
| `guard-read` | `PreToolUse` | `Read` | Block lockfiles, generated files, huge files without `offset`/`limit`, and unchanged re-reads | F3 |
| `guard-search` | `PreToolUse` | `Grep\|Glob` | Deny overly broad searches in large repositories | F3 |
| `session-check` | `Stop` | – | Warn the user (never the model) when the context passes `context_warn_tokens` | F5 |
| `session-start` | `SessionStart` | – | Register the session; on `clear`/`compact` mark a cut so re-reads are allowed again | F3 |

> **Phase F0:** all six are installed but are fail-open stubs that drain stdin and print nothing, so configuring early never breaks anything.

The subagent tool is called `Agent` (it was `Task` before Claude Code v2.1.63, and `Task` still works as an alias), so `enforce` matches both.

## Entry format

Each tokred hook goes in a **group of its own**, never mixed with rtk's or yours:

```json
{ "matcher": "Read",
  "hooks": [{ "type": "command", "command": "\"/opt/homebrew/bin/tokred\" hook guard-read", "timeout": 5 }] }
```

tokred recognizes its own entries by the command: an executable named `tokred` (optionally quoted, `tokred.exe` on Windows) followed by `hook <name>`. Only those entries are ever added, replaced or removed.

## Modes

Every hook honours the mode set in `[modes]` of `tokred.toml`: `off` (does nothing), `warn` (allows and advises) or `enforce` (denies). Defaults: `guard_read = "enforce"`, everything else `"warn"`.

## Guarantees

- **Fail-open:** on any error, panic or timeout a hook exits 0 with no output and logs the problem to `<data>/logs/`. tokred can never block Claude Code because of its own bug.
- **Fast:** each run should stay under ~50 ms (p95). No network inside a hook, ever.
- **Minimal context:** any `additionalContext` is one or two lines and is only emitted when something changed.
- **Unknown hook names exit 0** (for example after a downgrade), so Claude Code never sees exit code 2, which means "block".

## Coexistence with rtk

rtk acts on `PreToolUse` with the `Bash` matcher; tokred never registers a `Bash` matcher. `tokred doctor` detects the rtk hooks, confirms there is no overlap, and notes any other hook that shares a matcher with a tokred hook.
