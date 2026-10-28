**English** | [Português](decisions.pt-BR.md)

# Decision log

Format: date, context, decision, consequences. Where the spec diverges from the official Claude Code documentation, the documentation wins and the divergence is recorded here.

## 2026-10-28 — Subagent tool is `Agent`, matcher `Agent|Task`

- **Context:** the spec says to confirm whether the subagent tool is `Task` or `Agent` (section 7.2).
- **Decision:** per the sub-agents documentation the tool is `Agent`, renamed from `Task` in Claude Code v2.1.63, with `Task(...)` still accepted as an alias. The `enforce` hook uses the matcher `Agent|Task` (the matcher syntax accepts `|`-separated exact names).
- **Consequences:** works on both old and new Claude Code versions.

## 2026-10-28 — Hook `timeout` is in seconds

- **Context:** the hooks documentation defines `timeout` in seconds (default 600 for command hooks).
- **Decision:** every tokred hook is registered with `timeout: 5`, as in the spec's example.

## 2026-10-28 — tokred hook commands are recognized structurally, not by the substring `tokred hook `

- **Context:** spec section 5.1 says a tokred entry's `command` "contains the marker `tokred hook `", but its own example is `"/opt/homebrew/bin/tokred" hook guard-read`: the closing quote sits between `tokred` and `hook`, so that substring never occurs in the entries tokred itself writes. A literal implementation would never find (and so never replace or remove) its own entries.
- **Decision:** a command is tokred's when it runs an executable whose file name starts with `tokred` (`tokred`, `tokred.exe`, or a versioned/renamed copy such as `tokred-0.4.0`), optionally quoted, followed by ` hook <name>`. Found by a unit test: under `cargo test` the binary is called `tokred-<hash>`, and an exact-name rule made `configure` add duplicates and `unconfigure` remove nothing.
- **Consequences:** idempotency and unconfigure work for quoted paths, Windows paths with spaces, renamed binaries and bare `tokred hook x`. A wrapper such as `bash -c 'tokred hook x'` is deliberately not recognized.

## 2026-10-28 — Return-format labels are in English

- **Context:** the spec's return format uses `RESULTADO/ARQUIVOS/PENDÊNCIAS`, but section 0 requires agent templates in English.
- **Decision:** the labels are `RESULT`, `FILES` and `PENDING`.

## 2026-10-28 — Documentation file names are lowercase

- **Context:** the spec says `docs/DECISIONS.md` in section 0 and `docs/decisions.md` in section 16.2.
- **Decision:** lowercase, with the `.pt-BR.md` twin. The spec translation (`docs/spec.md`) keeps the original wording.

## 2026-10-28 — Hooks are fail-open stubs in F0

- **Context:** F0 registers the section 7 hooks but their logic arrives in F3 to F5; a user who configures in between must never be broken.
- **Decision:** `tokred hook <event>` drains stdin and prints nothing. `hook` is dispatched **before** argument parsing and always exits 0, even for an unknown event, because Claude Code treats exit code 2 as "block".

## 2026-10-28 — Minimum Rust version is 1.89

- **Context:** edition 2024 needs 1.85, but `rusqlite` 0.41 declares 1.89.
- **Decision:** `rust-version = "1.89"`, verified with `cargo +1.89 check --locked` locally and in CI.

## 2026-10-28 — Data folder from `BaseDirs`, not `ProjectDirs`

- **Context:** the spec lists the data folders per OS and names `directories::ProjectDirs`. On Windows `ProjectDirs` appends `\data` (`%APPDATA%\tokred\data`), which does not match the spec's `%APPDATA%\tokred`.
- **Decision:** use `BaseDirs::data_dir().join("tokred")`, which gives exactly the three paths in the spec.

## 2026-10-28 — `option-ext` (MPL-2.0) allowed by a per-crate exception

- **Context:** `directories` → `dirs-sys` → `option-ext`, licensed MPL-2.0, which is not in the spec's allow-list (MIT, Apache-2.0, BSD, ISC, Unicode, Zlib).
- **Decision:** keep `directories` (the spec's choice) and add a `cargo-deny` exception for that one crate. MPL-2.0 is file-level copyleft and the crate is used unmodified.

## 2026-10-28 — Git remote kept as the existing SSH alias

- **Context:** the spec says `git@github.com:sostenes198/tokred.git`; the repository already had `git@github.com-pessoal:sostenes198/tokred.git`.
- **Decision:** keep the existing remote (the author's choice: use the SSH configuration on their machine).

## 2026-10-28 — Details of `configure` / `unconfigure` not fixed by the spec

- `--claude-dir` is accepted by `configure`, `unconfigure` and `doctor` (the spec lists it only for `configure`); an explicit but invalid path is an error and does not fall back to other candidates.
- `configure` also takes `--force` (the spec mentions it for `update`).
- A blank `settings.json` counts as `{}`; invalid JSON aborts with nothing written, no backup and no data folder created.
- `settings.json` is only rewritten when the merged result differs, and is re-serialized with 2-space indentation (key order is preserved, original whitespace is not). The backup holds the exact previous text.
- Round-trip limit: an event array or `hooks` object that was **already empty** and that tokred merged into is removed by `unconfigure`, because "emptied by our removal" cannot be told apart from "empty to begin with". Empty containers tokred did not touch are preserved.
- If `settings.json` did not exist, `unconfigure` leaves `{}` behind.
- `unconfigure` keeps agent files you edited (and, when the database is gone, every agent file, since their origin cannot be verified).
- `--purge` refuses to delete a folder that has neither `tokred.db` nor `tokred.toml`.
- The database is created only after the plan is confirmed.
- `tokred.toml` is rewritten (without comments) only when keys were missing; otherwise it is left byte for byte.
- `unconfigure` is lenient: it removes tokred's handlers and leaves anything it does not understand (a non-object `hooks`, an event that is not an array) exactly as it is. Only `configure`, which has to write into those places, reports a bad shape, and only for the events it writes to.
- Terminal I/O goes through a small `Io` seam (interactive flag, input, output), and command output is rendered by pure functions into `String`s. This is what lets the prompts, the plan text and every `doctor` state be unit-tested without a TTY, and what keeps line coverage at 100%.
- `doctor --bench`, price sync and `tokred metrics` are not part of F0 and are omitted from the quick start until their phases.
