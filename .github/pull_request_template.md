## What and why

<!-- What changes, and the reason for it. Link the issue it closes, if any. -->

## Checklist

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo test --workspace` pass
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` and `cargo deny check` pass
- [ ] New behaviour has tests (a bug fix includes a test that reproduces the bug)
- [ ] Docs are updated in **both** languages (`X.md` and `X.pt-BR.md`)
- [ ] `CHANGELOG.md` and `CHANGELOG.pt-BR.md` are updated for user-visible changes
- [ ] Commits follow Conventional Commits and are in English

See [CONTRIBUTING.md](../CONTRIBUTING.md).
