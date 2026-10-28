**English** | [Português](agents.pt-BR.md)

# Subagents

`tokred configure` installs six subagents into `<claude>/agents/`, each with a fixed model:

| File | Model | Tools | Use |
|---|---|---|---|
| `tokred-architect.md` | opus | Read, Grep, Glob | Architecture, design decisions, trade-offs |
| `tokred-planner.md` | opus | Read, Grep, Glob | Planning and breaking down large tasks |
| `tokred-dev.md` | sonnet | Read, Edit, Write, Grep, Glob, Bash | Implementation, refactoring, bug fixes |
| `tokred-reviewer.md` | sonnet | Read, Grep, Glob, Bash | Code and test review |
| `tokred-explorer.md` | haiku | Read, Grep, Glob | Codebase search (read-only) |
| `tokred-summarizer.md` | haiku | Read, Grep, Glob | Summaries of docs, logs and long output (read-only) |

The templates live in `templates/agents/` and are written in English. Frontmatter: `name`, `description` (when to delegate), `model`, `tools`.

## Common rules and return format

Every agent is told not to restate the task or describe its process, to prefer targeted edits over rewriting files, and to reply **only** in this short format, which keeps tokens low and the result returned to the main context small:

```
RESULT: <1-3 lines>
FILES: <paths changed or relevant>
PENDING: <list or "none">
```

## Managed files

- Files are named with the `tokred-` prefix. The first body line after the frontmatter is `<!-- managed-by: tokred vX.Y.Z -->`.
- The `installed_files` table records each file's path, blake3 hash and version.
- On update: if the file on disk **equals** the recorded hash it is overwritten; if it **differs** (you edited it) it is left alone and the new version is written to `tokred-x.md.new` next to it, and `tokred doctor` says so. `--force` overwrites anyway.
- Files without the prefix are **never** touched. `tokred unconfigure` keeps agents you edited.
