**English** | [Português](CONTRIBUTING.pt-BR.md)

# Contributing to tokred

## Workflow

1. Branch from `main` (`feat/...`, `fix/...`, `docs/...`).
2. Make small, focused commits using [Conventional Commits](https://www.conventionalcommits.org/) **in English** (`feat:`, `fix:`, `docs:`, `chore:`...).
3. Open a pull request against `main`; CI must be green on Linux, macOS and Windows.

## Language rules

- Code is in English: file, crate, module, type, function and variable names, database tables and columns, config keys, comments, commit messages and every string the CLI or the hooks print.
- Documentation is **bilingual**: each document has an English version (`X.md`) and a Portuguese version (`X.pt-BR.md`) with the same content. Update both in the same commit.

## Checklist before opening a PR

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
```

- [ ] New behaviour has tests; a bug fix includes a test that reproduces the bug.
- [ ] No test touches the real `~/.claude` or the real data folder (use temporary directories, `TOKRED_DATA_DIR` and `--claude-dir`).
- [ ] No test uses the network.
- [ ] Docs updated in both languages.
- [ ] `CHANGELOG.md` and `CHANGELOG.pt-BR.md` updated for user-visible changes.

## Architecture rules

Business rules are pure functions in `tokred-core` and the use-case crates; I/O lives behind the traits (ports) of `tokred-core` and is composed only in `tokred-cli`. See [docs/architecture.md](docs/architecture.md) and [docs/development.md](docs/development.md).

Before implementing or changing a hook, check the official hooks documentation (<https://code.claude.com/docs/en/hooks>) and record any divergence from the spec in [docs/decisions.md](docs/decisions.md).
