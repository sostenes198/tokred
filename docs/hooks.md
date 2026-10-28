**English** | [Português](hooks.pt-BR.md)

# Hooks

`tokred configure` registers six hooks in `settings.json`. Each one runs `"<path>/tokred" hook <name>` with a 5 second timeout (Claude Code's `timeout` is in **seconds**). Input and output formats follow the official reference, <https://code.claude.com/docs/en/hooks>.

| Hook | Claude Code event | Matcher | Purpose | Phase |
|---|---|---|---|---|
| `route` | `UserPromptSubmit` | – | Classify the prompt and suggest the right subagent (one line, only when the class changed) | F4 |
| `enforce` | `PreToolUse` | `Agent\|Task` | Compare the chosen subagent with the last classification | F4 |
| `guard-read` | `PreToolUse` | `Read` | Block lockfiles, generated files, huge files without `offset`/`limit`, and unchanged re-reads | F3 |
| `guard-search` | `PreToolUse` | `Grep\|Glob` | Deny overly broad searches in large repositories | F3 |
| `session-check` | `Stop` | – | Warn the user (never the model) when the context passes `context_warn_tokens` | F5 |
| `session-start` | `SessionStart` | – | Register the session; on `clear`/`compact` mark a cut so re-reads are allowed again | F3 |

The subagent tool is called `Agent` (it was `Task` before Claude Code v2.1.63, and `Task` still works as an alias), so `enforce` matches both.

## Entry format

Each tokred hook goes in a **group of its own**, never mixed with rtk's or yours:

```json
{ "matcher": "Read",
  "hooks": [{ "type": "command", "command": "\"/opt/homebrew/bin/tokred\" hook guard-read", "timeout": 5 }] }
```

tokred recognizes its own entries by the command: an executable whose file name starts with `tokred` (optionally quoted; `tokred.exe` on Windows; a versioned copy like `tokred-0.4.0` also counts) followed by `hook <name>`. Only those entries are ever added, replaced or removed.

## Modes

Every hook honours the mode set in `[modes]` of `tokred.toml`: `off` (does nothing and records nothing), `warn` (advises, the call goes ahead) or `enforce` (denies). Defaults: `guard_read = "enforce"`, everything else `"warn"`. `session-start` has no mode: it only registers the session.

How each answer is sent to Claude Code:

| Answer | Output |
|---|---|
| Deny (`enforce`) | `{"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": "<short reason>"}}` |
| Advise (`warn`) | `{"hookSpecificOutput": {"hookEventName": "<event>", "additionalContext": "<one line>"}}`, with no `permissionDecision` |
| Allow | No output (exit 0) |
| Message for the user (`session-check`) | `{"systemMessage": "<text>"}` |

tokred never prints `permissionDecision: "allow"`: Claude Code documents that it **skips the permission prompt**, and its reason only reaches the debug log. "Allow with a short reason" is therefore sent as `additionalContext`.

## What each hook decides

### `guard-read`

1. **Lockfiles and generated files.** `deny_globs` (defaults: `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `Cargo.lock`, `poetry.lock`, `*.min.js`, `*.map`, `dist/`, `build/`, `node_modules/`, `target/`, `.git/`) are matched against the path **relative to the session's `cwd`**, so a project that merely lives under a folder named `build` is not blocked. Invalid globs are ignored.
2. **Large file.** Above `max_read_bytes` with neither `offset` nor `limit`: denied, asking for a ranged read. Images and PDFs (`png`, `jpg`, `jpeg`, `gif`, `webp`, `bmp`, `pdf`) are exempt: `Read` shows them natively.
3. **Re-read.** The file's blake3 hash is compared with earlier reads of the same session for the same `path`, `offset` and `limit`; an identical one is denied (the content is already in context). Files above 16 MiB are not hashed (the check is skipped). A `clear` or `compact` (`session-start`) is a cut: only reads **strictly after** the cut count, so a read in the same second is allowed (the fail-open side).
4. The read and the bytes avoided are recorded (`bytes_saved`; in `warn` mode nothing is avoided, so 0). The size of a ranged read is estimated at 80 bytes per requested line.

Hooks fired inside a **subagent** carry an `agent_id`; their reads are tracked per `session/agent`, because the subagent's context does not contain what its parent read.

### `guard-search`

A `Grep` without `path`, `glob` or `type`, or a `Glob` whose pattern matches everything (`*`, `**`, `**/*`) and has no `path`, is "broad". Only broad searches count files: the repository root is the nearest ancestor with a `.git` (else the search directory), the files under it are counted (skipping `.git`, `node_modules`, `target`, `dist`, `build`, `.venv`, `__pycache__`, `.next`) up to `large_repo_files + 1`, and the count is kept in the [tiered cache](cache.md) keyed by root, threshold and the root's modification time. Above `large_repo_files` the search is denied (`enforce`) or advised against (`warn`, the default).

### `route`

A heuristic classifier (no LLM) over pt-BR and English keywords (architecture/design, plan/planning, implement/fix/bug/refactor, review, where is/find/search, summarize), the prompt length and file mentions. Prompts shorter than 30 characters that mention no file are one-off tasks and never get a suggestion. A one-line `additionalContext` is emitted only when the class changed since the last actionable prompt of the session, for example:

```
tokred: implementation task -> delegate to the tokred-dev subagent if it involves more than one edit.
```

Every run records a `route` event whose `decision` is the class (`architecture`, `planning`, `implementation`, `review`, `search`, `summary`, `other`). The `classifications` table only stores actionable classes, so an "ok, go ahead" between two implementation prompts does not make the suggestion repeat. `route` cannot deny: `enforce` mode behaves like `warn`.

### `enforce`

For the subagent tool, compares the chosen `subagent_type` with the session's last classification. Agents that are not tokred's are always allowed, as is a session with nothing classified. Architecture and planning accept either opus agent. On a mismatch, `warn` adds a short advisory and `enforce` denies, naming the right agent. Only the main conversation is checked (hooks with an `agent_id` are skipped).

### `session-start`

Registers the session and the working directory. On `source` = `clear` or `compact` it marks the cut used by `guard-read`. `startup`, `resume` and `fork` keep the earlier context, so they are not cuts. It prints nothing: on `SessionStart`, anything on stdout would be added to the model's context.

### `session-check`

Takes the `usage` of the last main-conversation assistant message in the transcript (`transcript_path`; only the tail of the file is read) and estimates the context as `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`. Above `context_warn_tokens` it prints a `systemMessage` for the **user** suggesting `/compact` or `/clear`. It never blocks the stop, never uses `decision` or `additionalContext` on `Stop` (that would make Claude continue) and warns once per session: again only after the context grew by half the limit, or after a `clear`/`compact`. The warning state lives in the tiered cache (4 h L1 entry). Claude Code writes the transcript asynchronously, so the figure can lag by one message.

## Guarantees

- **Fail-open:** on any error, panic, bad input or timeout a hook exits 0 with no output and logs the problem to `<data>/logs/`. A time guard (2 s, `TOKRED_HOOK_GUARD_MS` overrides it for tests) records `timeout_guard` and exits 0 if a hook gets stuck. If the data folder or `tokred.db` does not exist, a hook does nothing and never creates them. An invalid `tokred.toml` means the defaults.
- **Fast:** each run stays well under ~50 ms (p95): see `tokred doctor --bench`. No network inside a hook, ever.
- **Measured:** every run is recorded in `hook_events` with `duration_us` (measured from the first line of `main`) and `outcome` (`ok`, `error`, `panic`, `timeout_guard`). The write is the **last** step, after the answer was written and flushed. The process cannot close its own stdout (`unsafe` is forbidden), so Claude Code sees the end of the run only when the process exits; the write itself takes a fraction of a millisecond.
- **Minimal context:** any `additionalContext` is one line and only emitted when something changes.
- **Unknown hook names exit 0** (for example after a downgrade), so Claude Code never sees exit code 2, which means "block".

## Coexistence with rtk

rtk acts on `PreToolUse` with the `Bash` matcher; tokred never registers a `Bash` matcher. `tokred doctor` detects the rtk hooks, confirms there is no overlap, and notes any other hook that shares a matcher with a tokred hook.
