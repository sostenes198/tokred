**English** | [Português](release.pt-BR.md)

# Running locally and publishing a release

Two parts: [run tokred on your machine](#1-run-it-locally), then [ship a binary people install with one command](#2-publish-a-release). Contributor checks (tests, coverage, lints) are in [development](development.md).

## 1. Run it locally

### Build and run from the source tree

```sh
cargo build                       # debug build
cargo run -q -- doctor            # run any command: cargo run -q -- <command>
cargo build --release             # optimised binary at target/release/tokred
```

Requirements: Rust stable (MSRV 1.89; `rust-toolchain.toml` installs `rustfmt` and `clippy`). A binary built this way works from wherever it is, but `tokred update` will say to reinstall with the script (only a binary in the install folder updates itself).

### Try it without touching your real setup

Use a throwaway sandbox so nothing is written to your real `~/.claude` or data folder:

```sh
export TOKRED_DATA_DIR=$(mktemp -d) CLAUDE_CONFIG_DIR=$(mktemp -d) TOKRED_OFFLINE=1
mkdir "$CLAUDE_CONFIG_DIR/projects"
cargo run -q -- configure --yes   # hooks, agents and the database go to the sandbox
cargo run -q -- doctor            # check the result
cargo run -q -- uninstall --yes --purge
```

`TOKRED_OFFLINE=1` turns off the version check and the price sync.

### Try the install scripts without a release

The scripts download from `TOKRED_RELEASE_BASE_URL`, which accepts a `file://` folder. Build the archive the way `dist` lays it out and install from it into a throwaway home:

```sh
cargo build
sh tests/installer/install_test.sh      # does exactly that, in a sandbox, and checks the result
```

By hand: put `tokred-<target>/tokred` in `tokred-<target>.tar.gz`, write `<sha256>  tokred-<target>.tar.gz` to `tokred-<target>.tar.gz.sha256`, then

```sh
HOME=$(mktemp -d) TOKRED_HOME=$(mktemp -d) TOKRED_DATA_DIR=$(mktemp -d) CLAUDE_CONFIG_DIR=$(mktemp -d) \
  TOKRED_RELEASE_BASE_URL=file:///path/to/folder sh install/install.sh --yes --port 23456
```

On Windows, `tests\installer\install_test.ps1` does the same (it edits your user PATH and PowerShell profile and restores them; it asks for `-AllowRealProfile` outside CI).

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
dist plan                          # what a release would contain (archives, checksums, install scripts)
dist build --artifacts=local       # build the archives for this machine's target
dist build --artifacts=global      # the checksums and the install scripts
```

`dist` is installed with `cargo install cargo-dist --locked` (or `brew install cargo-dist`, a developer tool; users never need it); the version used in CI is pinned in `dist-workspace.toml`.

## 2. Publish a release

### How delivery works

One tag push does everything. The `dist`-generated `.github/workflows/release.yml` builds, for macOS, Linux (musl) and Windows on x86_64 and aarch64, an archive plus its `.sha256`, and uploads them to a GitHub Release together with the four install scripts copied from `install/`:

| Asset | What it is |
|---|---|
| `tokred-<target>.tar.gz` / `.zip` | The binary with the README, licences and changelog |
| `tokred-<target>.tar.gz.sha256` / `.zip.sha256` | `<sha256>  <file name>`; the install scripts and `tokred update` verify it |
| `install.sh`, `install.ps1`, `uninstall.sh`, `uninstall.ps1` | The scripts users run (`extra-artifacts` in `dist-workspace.toml`) |
| `source.tar.gz`, `sha256.sum` | Added by `dist` |

That is all: **no tokens, no secrets, no tap or bucket repositories, no package registry.** The pipeline needs nothing but the repository's own `GITHUB_TOKEN`. Users then run:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/install.sh | sh
irm https://github.com/sostenes198/tokred/releases/latest/download/install.ps1 | iex
```

(`releases/latest/download/` always points at the newest stable release; `releases/download/vX.Y.Z/` at a fixed one. The scripts also exist on the `main` branch at `https://raw.githubusercontent.com/sostenes198/tokred/main/install/`.) See [installation](installation.md) for what they do.

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

**Dry run first.** Push a pre-release tag such as `v0.2.0-rc.1`: `dist` builds everything and creates a GitHub *pre-release*. It is not "latest", so `releases/latest/download/` does not point at it and nobody gets it by accident. Try it with a fixed version:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/download/v0.2.0-rc.1/install.sh | sh -s -- --version 0.2.0-rc.1
```

Check `tokred --version`, `tokred doctor` and the uninstall script, then push the real tag (delete the pre-release if you want it gone).

### Check the result

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/install.sh | sh
tokred --version && tokred doctor
tokred update                     # "up to date", or it replaces itself
```

On Windows: `irm https://github.com/sostenes198/tokred/releases/latest/download/install.ps1 | iex`, then `tokred --version`.

### When something goes wrong

- **A build job fails:** fix, delete the tag (`git push origin :refs/tags/v0.2.0` and `git tag -d v0.2.0`) and tag again. Nothing was published if the failure was before the `host` job.
- **A bad release is out:** publish a new patch version; `latest` then points to it. To pull the bad one, delete the GitHub release (the tag can stay or go).
- **An install script has a bug:** fix it on `main` and cut a new patch release; the raw `main` URL serves the fix at once for people who use it.

### What to update when `dist-workspace.toml` changes

Run `dist generate` and commit the changed `.github/workflows/release.yml`; do not edit that file by hand. Dependabot does bump action versions inside it (for example `actions/checkout`); `allow-dirty = ["ci"]` in `dist-workspace.toml` makes `dist plan` accept that instead of failing with "out of date contents", so such pull requests can be merged (but it also makes `dist generate` refuse to run: comment the line out while you regenerate, then put it back). See also [installation](installation.md) and [decisions](decisions.md).
