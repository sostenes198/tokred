**English** | [Português](release.pt-BR.md)

# Running locally and publishing a release

Two parts: [run tokred on your machine](#1-run-it-locally), then [ship a binary people can install with Homebrew, Scoop, winget, `cargo install` or a script](#2-publish-a-release). Contributor checks (tests, coverage, lints) are in [development](development.md).

## 1. Run it locally

### Build and run from the source tree

```sh
cargo build                       # debug build
cargo run -q -- doctor            # run any command: cargo run -q -- <command>
cargo build --release             # optimised binary at target/release/tokred
cargo install --path crates/tokred-cli   # install it as `tokred` in ~/.cargo/bin
```

Requirements: Rust stable (MSRV 1.89; `rust-toolchain.toml` installs `rustfmt` and `clippy`).

### Try it without touching your real setup

Use a throwaway sandbox so nothing is written to your real `~/.claude` or data folder:

```sh
export TOKRED_DATA_DIR=$(mktemp -d) CLAUDE_CONFIG_DIR=$(mktemp -d) TOKRED_OFFLINE=1
mkdir "$CLAUDE_CONFIG_DIR/projects"
cargo run -q -- configure --yes   # hooks, agents and the database go to the sandbox
cargo run -q -- doctor            # check the result
cargo run -q -- unconfigure --purge
```

`TOKRED_OFFLINE=1` turns off the version check and the price sync.

### Run the proxy

The [proxy](proxy.md) is optional. With the real binary (not `cargo run`, so the background process outlives the build):

```sh
tokred proxy start                # background; or `tokred proxy start --foreground` to watch it
tokred proxy status               # running? does this terminal use it?
tokred claude                     # Claude Code through the proxy, for this run only
tokred routing status             # decisions recorded so far (routing is `observe` by default)
tokred stats                      # figures, including suggestions
tokred proxy stop
```

`tokred claude` starts the proxy by itself if it is not running, passes every argument to `claude` and exits with its exit code. If the proxy cannot start, Claude Code runs as before.

To start the proxy at login: `tokred proxy service install` (undo with `uninstall`).

### Build the release archives on your machine

This is what the release pipeline does, without publishing anything:

```sh
dist plan                          # what a release would contain (archives, installers, checksums)
dist build --artifacts=local       # build the archives for this machine's target
```

`dist` is installed with `cargo install cargo-dist --locked` (or `brew install cargo-dist`); the version used in CI is pinned in `dist-workspace.toml`.

## 2. Publish a release

### How delivery works

One tag push does everything. The `dist`-generated `.github/workflows/release.yml` builds, for macOS, Linux (musl) and Windows on x86_64 and aarch64, an archive plus its `.sha256`, a shell installer, a PowerShell installer and a Homebrew formula. Then `publish-package-managers.yml` updates the other channels:

| Channel | What gets published | Where | Secret used |
|---|---|---|---|
| GitHub Release | archives, checksums, installers | `sostenes198/tokred` releases | `GITHUB_TOKEN` (automatic) |
| Homebrew | formula `tokred.rb`, plus the `tokred configure` caveat | repo `sostenes198/homebrew-tap` | `HOMEBREW_TAP_TOKEN` |
| Scoop | manifest `bucket/tokred.json` | repo `sostenes198/scoop-bucket` | `SCOOP_BUCKET_TOKEN` |
| winget | pull request to `microsoft/winget-pkgs` | package `sostenes198.tokred` | `WINGET_TOKEN` |
| crates.io | every crate of the workspace (`cargo publish --workspace`) | crates.io | `CARGO_REGISTRY_TOKEN` |
| Script | `tokred-installer.sh` / `tokred-installer.ps1` | attached to the GitHub Release | none |

Users then run:

```sh
brew install sostenes198/tap/tokred
scoop bucket add sostenes198 https://github.com/sostenes198/scoop-bucket && scoop install sostenes198/tokred
winget install sostenes198.tokred
cargo install tokred
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/tokred-installer.sh | sh
```

None of these touches Claude Code; the user still runs `tokred configure`.

### One-time setup

The pipeline has never run yet, and none of this exists until you create it. Do it once:

1. **GitHub account.** The `gh` CLI on this machine may be logged into the work account; the repositories below belong to the personal one (`sostenes198`). Check with `gh auth status`, switch with `gh auth switch`.
2. **Homebrew tap.** Create the public repo `sostenes198/homebrew-tap` (it may be empty; `dist` creates `Formula/tokred.rb`). The `homebrew-` prefix is what makes `brew install sostenes198/tap/tokred` work.
3. **Scoop bucket.** Create the public repo `sostenes198/scoop-bucket`.
4. **Tokens** (GitHub → Settings → Developer settings → Personal access tokens), stored as repository secrets of `sostenes198/tokred` (Settings → Secrets and variables → Actions):
   - `HOMEBREW_TAP_TOKEN`: fine-grained token, repository `homebrew-tap`, permission *Contents: read and write*.
   - `SCOOP_BUCKET_TOKEN`: the same, for `scoop-bucket`.
   - `WINGET_TOKEN`: classic token with the `public_repo` scope.
   - `CARGO_REGISTRY_TOKEN`: create at <https://crates.io/settings/tokens> (needs a crates.io account linked to GitHub; scope *publish-new* and *publish-update*).
5. **winget, first time only.** Submit the first manifest to `microsoft/winget-pkgs` by hand (for example with `wingetcreate new <url of the Windows zip>`). After it is merged, the pipeline opens the update pull requests. Skip winget until you want it: remove the `winget` job from `publish-package-managers.yml` or the run will fail on that job (the other channels are not affected).
6. **crates.io names.** `tokred`, `tokred-core`, `tokred-proxy` and the rest are free today. Publishing is permanent: a version can be yanked but never deleted or reused. Check `cargo publish --workspace --dry-run` first.

### Every release

```sh
# 1. Start from a green main
git switch main && git pull
cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace

# 2. Bump the version: [workspace.package] and the `version = "…"` of every tokred-* entry
#    in [workspace.dependencies] must all match
sed -i '' 's/"0\.1\.0"/"0.2.0"/' Cargo.toml   # macOS; on Linux: sed -i 's/…/…/'
cargo check --workspace                         # refreshes Cargo.lock

# 3. CHANGELOG.md and CHANGELOG.pt-BR.md: turn [Unreleased] into [0.2.0] - YYYY-MM-DD
#    (the GitHub Release notes are taken from it)

# 4. Check what would be released
dist plan

# 5. Commit, then tag and push (this is the moment the release starts)
git commit -am "chore: release 0.2.0"
git tag v0.2.0
git push && git push origin v0.2.0
```

Watch the run in the repository's **Actions** tab (workflow *Release*). It takes about 10 to 20 minutes.

**Dry run first.** Push a pre-release tag such as `v0.2.0-rc.1`: `dist` builds everything and creates a GitHub *pre-release*, and by default does not run the publish jobs for pre-releases, so Homebrew, Scoop, winget and crates.io stay untouched. Confirm in the run that the publish jobs were skipped, download an archive from the pre-release and try it. Only then push the real tag.

### Check the result

```sh
brew update && brew install sostenes198/tap/tokred
tokred --version && tokred doctor
tokred update                     # shows the right command for how it was installed
```

On Windows: `scoop install sostenes198/tokred` or `winget install sostenes198.tokred`, then `tokred --version`.

### When something goes wrong

- **A build job fails:** fix, delete the tag (`git push origin :refs/tags/v0.2.0` and `git tag -d v0.2.0`) and tag again. Nothing was published if the failure was before the publish jobs.
- **Only one channel failed** (for example winget): re-run just the failed job from the Actions tab; the others are already out.
- **A bad release is out:** publish a new patch version. Yank a crates.io version with `cargo yank --version 0.2.0 tokred`; for Homebrew and Scoop, revert the commit in the tap or bucket.
- **Formula not found right after the release:** `brew update`; the caveat job waits up to 10 minutes for the formula to land in the tap.

### What to update when `dist-workspace.toml` changes

Run `dist generate` and commit the changed `.github/workflows/release.yml`; never edit that file by hand. See also [installation](installation.md) and [decisions](decisions.md).
