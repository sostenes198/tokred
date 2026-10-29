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
```

You need Rust 1.89 or newer. In the new folder, build with `cargo build`, run the tests with `cargo test --workspace` and try your build with `cargo run -- doctor`. More in [development](docs/development.md).

## Workflow

1. Branch from `main` (`feat/...`, `fix/...`, `docs/...`).
2. Make small, focused commits using [Conventional Commits](https://www.conventionalcommits.org/) **in English** (`feat:`, `fix:`, `docs:`, `chore:`...).
3. Open a pull request against `main`. CI must pass: it runs the tests on Linux, macOS and Windows, and the other checks below.

## Before you open a pull request

Run the quality gate described in [development](docs/development.md): formatting, Clippy, tests, the documentation build and `cargo deny check`. CI runs the same checks. Then go through this list:

- [ ] New behaviour has tests; a bug fix includes a test that reproduces the bug.
- [ ] No test touches the real `~/.claude` or the real data folder, or the network (use temporary directories, `TOKRED_DATA_DIR` and `--claude-dir`).
- [ ] Docs are updated in both languages, and `python3 scripts/check-docs.py` prints `documentation ok`.
- [ ] `CHANGELOG.md` and `CHANGELOG.pt-BR.md` are updated for user-visible changes.

## Language

- **Code is in English:** names of files, crates, modules, types, functions and variables, database tables and columns, config keys, comments, commit messages and every string the CLI or the hooks print.
- **Documentation is bilingual:** each document has an English version (`X.md`) and a Brazilian Portuguese version (`X.pt-BR.md`) with the same content. If you do not write Portuguese, open the pull request with the English text and ask for help with the other half.

## Architecture

Business rules are pure functions in `tokred-core` and the use-case crates; I/O lives behind traits (ports) and is composed only in `tokred-cli`. See [architecture](docs/architecture.md) and [development](docs/development.md).

Design decisions and their reasons are recorded in [decisions](docs/decisions.md). Add an entry when you make one.
