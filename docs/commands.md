**English** | [Português](commands.pt-BR.md)

# Commands

Implemented in phase F0: `configure`, `unconfigure`, `doctor`, and the internal `hook`. The other commands from the spec arrive with their phases: `update`, `stats`, `metrics`, `pricing`, `cache` (F1), `audit` (F2).

All three commands accept `--claude-dir <path>`.

## `tokred configure [--claude-dir <path>] [--yes] [--force]`

Integrates tokred with Claude Code.

1. Finds the Claude Code folder, in this order: `--claude-dir`, the value saved in `tokred.toml` (`[claude] dir`), `$CLAUDE_CONFIG_DIR`, `~/.claude` (`%USERPROFILE%\.claude` on Windows). A folder is valid when it exists and contains `settings.json` or `projects/`. An explicit `--claude-dir` that is not valid is an error. If nothing is found, an interactive terminal is asked for the path; otherwise the command fails and suggests `--claude-dir`.
2. Detects rtk and tells you the two will coexist.
3. Prints the **plan**: hooks to add (`+`), update (`~`) or keep (`=`), agents to create, update or leave alone, and the backup it will make. It asks for confirmation; `--yes` skips the question (required when stdin is not a terminal).
4. Applies it: creates the data folder, database and `tokred.toml`, merges `settings.json` (backup in `settings.json.tokred.bak`, only the last one is kept), and installs the agents.

Running it again is safe: the result is the same. If `settings.json` is not valid JSON the command aborts and **writes nothing**.

`--force` overwrites agent files even if you edited them (otherwise your version is kept and the new one is written to `tokred-x.md.new`).

## `tokred unconfigure [--claude-dir <path>] [--purge]`

Removes the tokred hooks from `settings.json` and the agent files tokred installed and you did not edit. Edited agents are kept and reported. Files without the `tokred-` prefix are never touched. `--purge` also deletes the data folder; it refuses to do so if the folder does not contain `tokred.db` or `tokred.toml`.

## `tokred doctor [--claude-dir <path>] [--json]`

Shows the tokred version, binary path, Claude Code and data folders, database schema version, config status, rtk detection, a table of hooks (`ok`, `missing`, `outdated`, or a binary path that no longer exists), a table of agents (`current`, `outdated`, `edited by you`, `missing`, `.new` pending), and a list of problems and notes.

With `--json` it prints one JSON object:

```json
{
  "tokred_version": "0.1.0",
  "binary": "/opt/homebrew/bin/tokred",
  "binary_on_path": true,
  "data_dir": "…",
  "database": true,
  "schema_version": 1,
  "config": true,
  "rtk_binary_on_path": true,
  "install": {
    "claude_dir": "…",
    "settings_exists": true,
    "settings_valid": true,
    "hooks": [{ "hook": "guard-read", "event": "PreToolUse", "state": "ok", "binary_missing": false }],
    "agents": [{ "file": "tokred-dev.md", "state": "current", "pending_new": false }],
    "rtk_hooks_detected": true,
    "issues": [],
    "notes": []
  }
}
```

`doctor --bench` (hook latency) arrives in phase F3.

## `tokred hook <event>` (internal)

Called by Claude Code, never by hand (except for testing, see [development](development.md)). Reads JSON on stdin, writes JSON on stdout. Events: `route`, `enforce`, `guard-read`, `guard-search`, `session-check`, `session-start`. It **always exits 0**, even for an unknown event or invalid input. In phase F0 every hook is a stub that prints nothing.
