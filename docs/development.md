**English** | [Português](development.pt-BR.md)

# Development

## Requirements

- Rust stable (the workspace MSRV is **1.89**, checked in CI) with `rustfmt` and `clippy` (`rust-toolchain.toml` installs them).
- For the full local gate: `cargo install cargo-deny cargo-llvm-cov`.

## Build

```sh
cargo build                          # debug build of every crate
cargo run -- doctor                  # run the CLI
cargo install --path crates/tokred-cli
```

## Checks (the same ones CI runs)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
cargo +1.89 check --workspace --all-targets --locked    # MSRV
```

## Tests

| Kind | Where | Covers |
|---|---|---|
| Unit | `#[cfg(test)]` inside each module | Pure functions: settings merge, agent install decision, config merge, hook name parsing |
| Integration | `crates/<crate>/tests/` | `tokred-store` on a real SQLite file; `tokred-install` configure/unconfigure/doctor against fixtures on real temp directories |
| End to end | `crates/tokred-cli/tests/` | The real binary via `assert_cmd`: `configure` → hooks → `doctor` → `unconfigure`, in a sandbox |
| Snapshot | alongside the above (`insta`) | Merged `settings.json`, `doctor --json` hooks |
| Property | `crates/tokred-install/tests/merge_props.rs` (`proptest`) | configure twice = once; configure + unconfigure = original; foreign hooks survive |

Rules: no test touches the real `~/.claude` or the real data folder, and no test uses the network. The end-to-end tests set `HOME`, `CLAUDE_CONFIG_DIR` and `TOKRED_DATA_DIR` to temporary directories (one of them with a space in its name). Every bug fix comes with a test that reproduces the bug.

Integration-test files start with `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` because clippy's `allow-*-in-tests` options only cover `#[cfg(test)]` code, not files under `tests/`.

### Snapshots

```sh
INSTA_UPDATE=always cargo test        # write new/changed snapshots
git diff -- '*.snap'                  # review them before committing
```

### Fixtures

`tests/fixtures/` holds `settings.json` variants (empty, with rtk hooks, with user hooks, with an old tokred version, invalid JSON, unknown keys). Add a fixture whenever you find a real-world shape that the merge must handle.

## Coverage

```sh
cargo llvm-cov --workspace --all-features --summary-only
```

Target: at least 80% in `tokred-core` and the use-case crates; the workspace is currently at **100% of lines** (every instrumented line runs) and 100% of functions.

To list lines that never run:

```sh
cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info   # DA:<line>,0 = never executed
cargo llvm-cov report --text                                               # annotated source, `^0` marks a region that never ran
```

Region coverage is lower (about 98%) because every `?` has an error branch; forcing each I/O failure would add noise, not safety. Prompts, the plan text and `doctor` output are unit-tested through the `Io` seam and pure render functions, so no test needs a terminal.

## Trying a hook locally

Hooks read JSON on stdin, so you can run one by hand against a sandbox:

```sh
export TOKRED_DATA_DIR=$(mktemp -d)
echo '{"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"Cargo.lock"}}' \
  | cargo run -q -- hook guard-read; echo "exit=$?"
```

Before implementing a hook, read the official docs (<https://code.claude.com/docs/en/hooks>) for the exact input and output format, and record any divergence from the spec in [decisions](decisions.md).

## Releases

The release pipeline (`dist`, Homebrew tap, scoop bucket, winget, `cargo publish`) is part of phase F1. Note for that phase: `tokred-install` embeds `templates/agents/*.md` with `include_str!` from the repository root, so those files must be packaged with the crate before `cargo publish` can work.
