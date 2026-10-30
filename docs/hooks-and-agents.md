**English** | [Português](hooks-and-agents.pt-BR.md)

# Hooks and agents

How tokred plugs into Claude Code without the proxy: seven hooks that run around your prompts and tool calls, and six subagents with a fixed model each. `tokred configure` installs both; `tokred unconfigure` removes them.

## The seven hooks

Each hook is registered in `settings.json` as `"<path>/tokred" hook <name>` with a 5 second timeout. Input and output formats follow Claude Code's [hooks reference](https://code.claude.com/docs/en/hooks).

| Hook | Claude Code event | Matcher | What it decides | Default mode |
|---|---|---|---|---|
| `route` | `UserPromptSubmit` | none | Classifies the prompt and suggests the right subagent (one line, only when the class changed) | `warn` |
| `enforce` | `PreToolUse` | `Agent\|Task` | Compares the subagent Claude chose with the last classification | `warn` |
| `guard-read` | `PreToolUse` | `Read` | Blocks lockfiles, generated files, huge files read whole and unchanged re-reads | `enforce` |
| `guard-search` | `PreToolUse` | `Grep\|Glob` | Denies overly broad searches in large repositories | `warn` |
| `session-check` | `Stop` | none | Warns you (never the model) when the context grows past a limit | `warn` |
| `session-start` | `SessionStart` | none | Registers the session; after `/clear` or `/compact` allows re-reads again | no mode |
| `model-guard` | `PreModelSwitch` | none | Tells you what a model switch costs in prompt cache, or asks you to confirm it | `warn` |

### Modes

Every hook except `session-start` honors its mode in `[modes]` of `tokred.toml`:

| Mode | Behavior |
|---|---|
| `off` | Does nothing and records nothing |
| `warn` | Advises; the call goes ahead |
| `enforce` | Denies the call (`on` is accepted as another name for it) |

To turn a hook off, set its mode to `"off"`; no need to touch `settings.json`:

```toml
[modes]
guard_read = "off"
```

The keys are `route`, `enforce`, `guard_read`, `guard_search`, `session_check` and `model_guard`. `route` and `session_check` never deny, so `enforce` behaves like `warn` for them. `model_guard` never denies either: with `"on"` it asks you to confirm the switch. The thresholds of each hook (`max_read_bytes`, `deny_globs`, `large_repo_files`, `context_warn_tokens`, `min_context_tokens`) are in [configuration](configuration.md).

How an answer reaches Claude Code:

| Answer | Output |
|---|---|
| Deny (`enforce`) | `permissionDecision: "deny"` with a short reason |
| Advise (`warn`) | `additionalContext`: one line, no decision |
| Allow | No output |
| Ask (`model-guard` with `on`) | `permissionDecision: "ask"`: Claude Code asks you to confirm the switch |
| Message for you (`session-check`, `model-guard`) | `systemMessage`; it never reaches the model |

tokred never sends `permissionDecision: "allow"`, because Claude Code would then skip its permission prompt (or, for a model switch, the confirmation it shows while the prompt cache is warm).

### What each hook decides

**`guard-read`**

1. **Lockfiles and generated files.** A path that matches `deny_globs` (relative to the session's folder) is denied. Defaults: `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `Cargo.lock`, `poetry.lock`, `*.min.js`, `*.map` and the folders `dist/`, `build/`, `node_modules/`, `target/`, `.git/`.
2. **Large file.** Above `max_read_bytes` with neither `offset` nor `limit`: denied, asking for a ranged read. Images and PDFs are exempt.
3. **Re-read.** A read of the same path, `offset` and `limit` whose content did not change since an earlier read of the same session is denied: the content is already in context. After `/clear` or `/compact` only reads made after that point count.

Subagents are tracked separately, because a subagent's context does not contain what its parent read. The bytes avoided are recorded; in `warn` mode nothing is avoided, so the saving is 0.

**`guard-search`**

A search that starts at the root of the repository (the nearest folder with a `.git`, or the session's folder) is broad when it is a `Grep` with no `glob` or `type`, or a `Glob` whose pattern matches every file (`*`, `**`, `**/*`, ...). Only broad searches are checked: the files of the repository are counted (the count is cached, and abandoned after 100 ms, which allows the search). Above `large_repo_files` the search is denied (`enforce`) or advised against (`warn`).

**`route`**

A heuristic classifier (no model call) over Portuguese and English keywords, the prompt length and file mentions. It sorts a prompt into `architecture`, `planning`, `implementation`, `review`, `search`, `summary` or `other`. Very short prompts that mention no file never get a suggestion. When the class changed since the last actionable prompt of the session it adds one line:

```text
tokred: implementation task -> delegate to the tokred-dev subagent if it involves more than one edit.
```

With the [proxy](proxy.md) running and `[suggest] channel = "message"`, `route` can also show you a one-line message that a cheaper model for the main loop would have done, because the recent prompts were simple. It appears once, only at a safe moment (the prompt cache is already cold, or nothing ran since the last `/compact`), and never changes the model.

**`enforce`**

Compares the subagent Claude chose with the session's last classification. Agents that are not tokred's are always allowed, and so is a session with nothing classified. On a mismatch `warn` adds a short advisory and `enforce` denies, naming the right agent. Only the main conversation is checked.

**`session-start`**

Registers the session and its folder, and marks the cut after `/clear` or `/compact` for `guard-read`. It prints nothing. It is also a safety net for the proxy: when `[proxy] enabled = true`, the session is pointed at the proxy and the proxy does not answer quickly, it starts the proxy in the background without waiting, so a proxy that died cannot lock Claude Code out.

**`session-check`**

Reads the token usage of the last main-conversation message in the transcript and estimates the context size. Above `context_warn_tokens` it tells you to consider `/compact` or `/clear`. It never blocks the stop and warns once per session: again only after the context grew by half the limit, or after a `/clear` or `/compact`. The figure can lag by one message.

**`model-guard`**

Runs before Claude Code applies a model change you asked for: `/model`, the model picker, the model setting in `/config` or fast mode. It needs Claude Code 2.1.251 or later, the version that added the `PreModelSwitch` event, and it does not run for changes Claude Code makes on its own, such as a fallback. Claude Code sends the size of the context, whether the prompt cache is still warm and the estimated price of writing the context to the new model's cache, so tokred estimates nothing. When the cache is warm and the context has at least `min_context_tokens` tokens (100,000 by default), `warn` shows you a one-line message and `on` asks you to confirm the switch, both with those figures. Any other switch (a small context, a cold cache, the model already in use) gets no answer, so Claude Code's own confirmation applies. `ask` only works in the interactive `/model`: on the other surfaces (`-p`, `/config`, the Agent SDK) Claude Code treats it as a refusal. A model change never gets a `deny`.

### Guarantees

- **Fail-open.** On any error, panic, bad input or timeout a hook exits `0` with no output and logs the problem to `<data>/logs/`. A time guard of 2 seconds ends a hook that gets stuck. If the data folder or the database does not exist, a hook does nothing and never creates them. An unparsable `tokred.toml` means the defaults. An unknown hook name also exits `0`, so Claude Code never sees exit code `2`, which means "block".
- **Local and quick.** A hook never uses the network. `tokred doctor --bench` measures how long each one takes, and `tokred stats latency` flags a hook whose p95 passes 50 ms.
- **Measured.** A run that decides something, and every failed run, is recorded with its duration and outcome (`ok`, `error`, `panic`, `timeout_guard`), after the answer was written. A hook that does nothing (its mode is `off`, or the call is not one it checks) records nothing. `tokred stats latency` reports it.
- **Minimal context.** Any `additionalContext` is one line and is emitted only when something changes.

## The six subagents

`tokred configure` writes six agent files to `<claude>/agents/`, each with a fixed model:

| File | Model | Tools | Use |
|---|---|---|---|
| `tokred-architect.md` | opus | Read, Grep, Glob | Architecture, design decisions, trade-offs |
| `tokred-planner.md` | opus | Read, Grep, Glob | Planning and breaking down large tasks |
| `tokred-dev.md` | sonnet | Read, Edit, Write, Grep, Glob, Bash | Implementation, refactoring, bug fixes |
| `tokred-reviewer.md` | sonnet | Read, Grep, Glob, Bash | Code and test review |
| `tokred-explorer.md` | haiku | Read, Grep, Glob | Codebase search (read-only) |
| `tokred-summarizer.md` | haiku | Read, Grep, Glob | Summaries of docs, logs and long output (read-only) |

Every agent is told not to restate the task or describe its process, to prefer targeted edits to rewriting files, and to reply only in this short format, which keeps the result returned to the main conversation small:

```text
RESULT: <1-3 lines>
FILES: <paths changed or relevant>
PENDING: <list or "none">
```

### Installing, updating and editing agents

- The files carry the `tokred-` prefix and a `<!-- managed-by: tokred vX.Y.Z -->` line. Files without the prefix are never touched.
- tokred remembers the hash of each file it wrote. On an update, a file that still matches is overwritten with the new version. A file you edited is kept, the new version is written next to it as `tokred-x.md.new`, and `tokred doctor` points it out. `tokred configure --force` overwrites your edits.

## What configure and unconfigure do to `settings.json`

`tokred configure` adds each tokred hook as a group of its own, with its own matcher, and changes nothing else in the file:

```json
{ "matcher": "Read",
  "hooks": [{ "type": "command", "command": "\"/path/to/tokred\" hook guard-read", "timeout": 5 }] }
```

tokred recognizes its entries by their command (an executable whose file name starts with `tokred`, such as `tokred`, `tokred.exe` or `tokred-0.4.0`, followed by `hook <name>`). Only those entries are ever added, replaced or removed; your hooks and anything tokred does not understand stay as they are. Before it changes the file, `configure` saves the previous one as `settings.json.tokred.bak` (only the last backup is kept). If `settings.json` is not valid JSON, `configure` stops and writes nothing. Running it again is safe.

`tokred unconfigure` removes tokred's hook entries and the agent files tokred installed. It also writes `settings.json.tokred.bak`, deletes pending `.new` files, and keeps an agent that you edited or whose origin it cannot verify, reporting each one. It leaves the proxy and its `ANTHROPIC_BASE_URL` alone, and an upgrade never puts the hooks back: only `tokred configure` does. Both commands are described in [commands](commands.md).

tokred keeps its hooks in groups of their own and never changes hooks that belong to other tools or to you, so it can be used alongside tools such as rtk; `tokred doctor` points out other hook tools it finds.
