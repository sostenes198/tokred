**English** | [Português](CONTRIBUTING.pt-BR.md)

# Contributing to tokred

Thank you for helping. This page explains how to contribute; please also read the [code of conduct](CODE_OF_CONDUCT.md). To report a vulnerability, follow [SECURITY.md](SECURITY.md) instead of opening a public issue.

## Ways to help

- **Issues:** report a bug or suggest an improvement with the issue templates. Say what you ran and on which OS, and run `tokred doctor --json` (remove anything private before pasting it).
- **Documentation:** fix a mistake, clarify a page, improve a translation.
- **Code:** pick an issue or open one to discuss a change before you write a large one.

## Set up

```sh
git clone https://github.com/sostenes198/tokred.git
cd tokred
cargo build
cargo test --workspace
cargo run -- doctor
```

You need Rust 1.89 or newer. More in [development](docs/development.md).

## Workflow

1. Branch from `main` (`feat/...`, `fix/...`, `docs/...`).
2. Make small, focused commits using [Conventional Commits](https://www.conventionalcommits.org/) **in English** (`feat:`, `fix:`, `docs:`, `chore:`...).
3. Open a pull request against `main`. CI must be green on Linux, macOS and Windows.

## Before you open a pull request

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
```

- [ ] New behaviour has tests; a bug fix includes a test that reproduces the bug.
- [ ] No test touches the real `~/.claude` or the real data folder, or the network (use temporary directories, `TOKRED_DATA_DIR` and `--claude-dir`).
- [ ] Docs are updated in both languages.
- [ ] `CHANGELOG.md` and `CHANGELOG.pt-BR.md` are updated for user-visible changes.

## Language

- **Code is in English:** names of files, crates, modules, types, functions and variables, database tables and columns, config keys, comments, commit messages and every string the CLI or the hooks print.
- **Documentation is bilingual:** each document has an English version (`X.md`) and a Brazilian Portuguese version (`X.pt-BR.md`) with the same content. If you do not write Portuguese, open the pull request with the English text and ask for help with the other half.

## Architecture

Business rules are pure functions in `tokred-core` and the use-case crates; I/O lives behind the traits (ports) of `tokred-core` and is composed only in `tokred-cli`. See [architecture](docs/architecture.md) and [development](docs/development.md).

Design decisions, and any place where the official Claude Code documentation made us diverge from a plan, are recorded in [decisions](docs/decisions.md). Add an entry when you make one.
