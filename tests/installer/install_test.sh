#!/bin/sh
# Tests for install/install.sh and install/uninstall.sh (macOS and Linux).
#
# A fake release (a real archive built from the just-built binary, in the layout `dist` uses,
# plus its .sha256) is served through TOKRED_RELEASE_BASE_URL=file://..., and everything runs
# against a throwaway HOME / TOKRED_HOME / data folder / Claude folder. Nothing real is touched.
#
#   cargo build -p tokred && sh tests/installer/install_test.sh
#
# TOKRED_TEST_BIN overrides the binary that goes into the fake release.

set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT_DIR=$(cd "$HERE/../.." && pwd)
BIN="${TOKRED_TEST_BIN:-$ROOT_DIR/target/debug/tokred}"
INSTALL_SH="$ROOT_DIR/install/install.sh"
UNINSTALL_SH="$ROOT_DIR/install/uninstall.sh"
BEGIN="# >>> tokred >>>"

[ -x "$BIN" ] || { printf 'build the binary first: cargo build -p tokred (looked for %s)\n' "$BIN" >&2; exit 1; }

WORK=$(mktemp -d)
# Sandboxes whose installs started a proxy (one "home|data|claude" line each): stopped on exit.
PROXIES="$WORK/proxy-sandboxes"
: >"$PROXIES"
stop_proxies() {
    while IFS='|' read -r p_home p_data p_claude; do
        [ -n "$p_home" ] || continue
        env -i PATH="$PATH" HOME="$p_home" TOKRED_DATA_DIR="$p_data" CLAUDE_CONFIG_DIR="$p_claude" \
            TOKRED_OFFLINE=1 "$BIN" proxy stop >/dev/null 2>&1 || true
    done <"$PROXIES"
}
trap 'stop_proxies; rm -rf "$WORK"' EXIT
trap 'exit 1' INT TERM HUP

ok() { printf 'ok - %s\n' "$*"; }
die() { printf 'not ok - %s\n' "$*" >&2; exit 1; }
assert_file() { [ -f "$1" ] || die "missing file: $1"; }
assert_missing() { [ ! -e "$1" ] || die "should not exist: $1"; }
assert_has() { grep -qF -- "$2" "$1" || die "$1 does not contain: $2"; }
assert_lacks() { if grep -qF -- "$2" "$1" 2>/dev/null; then die "$1 should not contain: $2"; fi; }
assert_blocks() {
    n=$(grep -cF -- "$BEGIN" "$1" || true)
    [ "$n" = "$2" ] || die "$1 has $n tokred blocks, expected $2"
}

# ------------------------------------------------------------ fake release

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) TARGET=aarch64-apple-darwin ;;
    Darwin-x86_64) TARGET=x86_64-apple-darwin ;;
    Linux-aarch64 | Linux-arm64) TARGET=aarch64-unknown-linux-musl ;;
    Linux-x86_64) TARGET=x86_64-unknown-linux-musl ;;
    *) die "unsupported test platform" ;;
esac
# The installer maps a Rosetta shell to the arm64 build; do the same here.
if [ "$TARGET" = "x86_64-apple-darwin" ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = "1" ]; then
    TARGET=aarch64-apple-darwin
fi
ASSET="tokred-$TARGET.tar.gz"

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; else shasum -a 256 "$1" | awk '{print $1}'; fi
}

REL="$WORK/release"
mkdir -p "$REL/pack/tokred-$TARGET"
cp "$BIN" "$REL/pack/tokred-$TARGET/tokred"
printf 'readme\n' >"$REL/pack/tokred-$TARGET/README.md"
tar -czf "$REL/$ASSET" -C "$REL/pack" "tokred-$TARGET"
printf '%s  %s\n' "$(sha256_of "$REL/$ASSET")" "$ASSET" >"$REL/$ASSET.sha256"

# ------------------------------------------------------------ sandboxes

PORT=$(awk 'BEGIN { srand(); print int(20000 + rand() * 30000) }')
N=0

# new_sandbox: a fresh HOME, TOKRED_HOME (empty S_TOKRED_HOME means "unset": ~/.tokred), data folder
# and Claude folder.
new_sandbox() {
    N=$((N + 1))
    S_ROOT="$WORK/s$N"
    S_HOME="$S_ROOT/home"
    S_TOKRED_HOME="$S_ROOT/tokred-home"
    S_DATA="$S_ROOT/data"
    S_CLAUDE="$S_ROOT/claude"
    S_SHELL=""
    # Installs run with --no-proxy (so no proxy is left running) unless a test sets this to 1.
    S_PROXY=0
    mkdir -p "$S_HOME" "$S_CLAUDE/projects"
}

tokred_home() { printf '%s' "${S_TOKRED_HOME:-$S_HOME/.tokred}"; }

# in_sandbox [VAR=value ...] COMMAND...: only the sandbox variables are visible to COMMAND.
in_sandbox() {
    # shellcheck disable=SC2086 # the ${VAR:+...} words are either empty or one NAME=value word
    env -i PATH="$PATH" HOME="$S_HOME" TOKRED_DATA_DIR="$S_DATA" \
        CLAUDE_CONFIG_DIR="$S_CLAUDE" TOKRED_OFFLINE=1 TOKRED_RELEASE_BASE_URL="file://$REL" \
        ${S_TOKRED_HOME:+TOKRED_HOME="$S_TOKRED_HOME"} ${S_SHELL:+SHELL="$S_SHELL"} "$@"
}

install_in_sandbox() {
    if [ "$S_PROXY" = "1" ]; then
        printf '%s|%s|%s\n' "$S_HOME" "$S_DATA" "$S_CLAUDE" >>"$PROXIES"
        in_sandbox sh "$INSTALL_SH" "$@" </dev/null >"$S_ROOT/install.out" 2>&1
    else
        in_sandbox sh "$INSTALL_SH" --no-proxy "$@" </dev/null >"$S_ROOT/install.out" 2>&1
    fi
}
# `--no-proxy` leaves the default port (1029) in tokred.toml, and a tokred proxy of the machine the
# test runs on may be listening there: point the sandbox at a port nothing uses before removing it.
uninstall_in_sandbox() {
    if [ -f "$S_DATA/tokred.toml" ]; then
        sed "s#127\.0\.0\.1:1029#127.0.0.1:$PORT#" "$S_DATA/tokred.toml" >"$S_ROOT/toml.tmp" \
            && cat "$S_ROOT/toml.tmp" >"$S_DATA/tokred.toml"
    fi
    in_sandbox sh "$UNINSTALL_SH" "$@" </dev/null >"$S_ROOT/uninstall.out" 2>&1
}

# ------------------------------------------------------------ 1. a plain install (default folder)

new_sandbox
S_TOKRED_HOME=""
printf 'export FOO=1\n' >"$S_HOME/.bashrc"
cp "$S_HOME/.bashrc" "$S_ROOT/bashrc.original"
install_in_sandbox --yes --port "$PORT" || { cat "$S_ROOT/install.out" >&2; die "the install failed"; }

INSTALLED="$S_HOME/.tokred/bin/tokred"
assert_file "$INSTALLED"
[ -x "$INSTALLED" ] || die "the installed binary is not executable"
[ "$("$INSTALLED" --version)" = "$("$BIN" --version)" ] || die "the installed binary reports another version"
ok "the binary is installed and runs"

assert_blocks "$S_HOME/.profile" 1
assert_blocks "$S_HOME/.bashrc" 1
assert_has "$S_HOME/.bashrc" "export FOO=1"
assert_has "$S_HOME/.profile" 'claude() { if command -v tokred >/dev/null 2>&1; then tokred initialize; fi; command claude "$@"; }'
assert_has "$S_HOME/.profile" 'export PATH="$HOME/.tokred/bin:$PATH"'
ok "the profiles got one block with PATH and the claude function"

assert_has "$S_CLAUDE/settings.json" "$INSTALLED"
assert_file "$S_CLAUDE/agents/tokred-dev.md"
ok "tokred configure ran and wrote the hooks and the agents"

assert_has "$S_ROOT/install.out" "Skipping the proxy (--no-proxy)"
assert_has "$S_DATA/tokred.toml" "enabled = false"
assert_lacks "$S_DATA/tokred.toml" "127.0.0.1:$PORT"
assert_lacks "$S_CLAUDE/settings.json" "ANTHROPIC_BASE_URL"
ok "--no-proxy leaves [proxy] and the Claude Code settings untouched (the port is not used)"

# ------------------------------------------------------------ 2. idempotent

cp "$S_HOME/.profile" "$S_ROOT/profile.once"
cp "$S_HOME/.bashrc" "$S_ROOT/bashrc.once"
install_in_sandbox --yes --port "$PORT" || die "the second install failed"
cmp -s "$S_HOME/.profile" "$S_ROOT/profile.once" || die ".profile changed on the second run"
cmp -s "$S_HOME/.bashrc" "$S_ROOT/bashrc.once" || die ".bashrc changed on the second run"
assert_blocks "$S_HOME/.profile" 1
ok "installing twice leaves the profiles byte for byte the same"

# ------------------------------------------------------------ 3. a new shell finds tokred

for shell_bin in sh bash dash zsh; do
    command -v "$shell_bin" >/dev/null 2>&1 || continue
    seen=$(env -i HOME="$S_HOME" PATH=/usr/bin:/bin "$shell_bin" -c '. "$HOME/.profile"; tokred --version')
    [ "$seen" = "$("$BIN" --version)" ] || die "$shell_bin: tokred is not on PATH after sourcing .profile ($seen)"
done
ok "a new shell has tokred on PATH"

# ------------------------------------------------------------ 4. the wrapper, end to end

FAKE="$WORK/fake"
mkdir -p "$FAKE/with-tokred" "$FAKE/claude-only"
cat >"$FAKE/with-tokred/claude" <<'EOF'
#!/bin/sh
{ echo claude; for a in "$@"; do printf 'arg:%s\n' "$a"; done; } >>"$LOG"
EOF
cat >"$FAKE/with-tokred/tokred" <<'EOF'
#!/bin/sh
echo "tokred $*" >>"$LOG"
EOF
cp "$FAKE/with-tokred/claude" "$FAKE/claude-only/claude"
chmod +x "$FAKE/with-tokred/claude" "$FAKE/with-tokred/tokred" "$FAKE/claude-only/claude"

cat >"$WORK/drive-with-tokred.sh" <<'EOF'
. "$HOME/.profile"
PATH="$FAKE_DIR/with-tokred:/usr/bin:/bin"
claude "a b" "it's" "" --flag "*" '$HOME' 'say "hi"'
command claude direct
EOF
cat >"$WORK/drive-claude-only.sh" <<'EOF'
. "$HOME/.profile"
PATH="$FAKE_DIR/claude-only:/usr/bin:/bin"
claude "still runs"
EOF
cat >"$WORK/expected-with-tokred.log" <<'EOF'
tokred initialize
claude
arg:a b
arg:it's
arg:
arg:--flag
arg:*
arg:$HOME
arg:say "hi"
claude
arg:direct
EOF
cat >"$WORK/expected-claude-only.log" <<'EOF'
claude
arg:still runs
EOF

for shell_bin in sh bash dash zsh; do
    command -v "$shell_bin" >/dev/null 2>&1 || continue
    for scenario in with-tokred claude-only; do
        LOG="$WORK/$shell_bin-$scenario.log"
        : >"$LOG"
        env -i HOME="$S_HOME" LOG="$LOG" FAKE_DIR="$FAKE" PATH=/usr/bin:/bin "$shell_bin" "$WORK/drive-$scenario.sh" ||
            die "$shell_bin: the wrapper script failed ($scenario)"
        cmp -s "$LOG" "$WORK/expected-$scenario.log" || {
            diff "$WORK/expected-$scenario.log" "$LOG" >&2 || true
            die "$shell_bin: the wrapper recorded the wrong calls ($scenario)"
        }
    done
done
ok "claude runs tokred initialize first, passes every argument through intact, and still runs without tokred"

# An alias from Claude Code's own installer must not break the profile.
if command -v zsh >/dev/null 2>&1; then
    printf 'alias claude="$HOME/.claude/local/claude"\n' >"$WORK/alias.zsh"
    cat "$WORK/alias.zsh" "$S_HOME/.profile" >"$WORK/zshrc-with-alias"
    env -i HOME="$S_HOME" PATH=/usr/bin:/bin zsh -c ". '$WORK/zshrc-with-alias'; whence -w claude" >"$WORK/alias.out" 2>&1 ||
        die "zsh could not load a profile that already defines an alias for claude"
    assert_has "$WORK/alias.out" "function"
    ok "an existing claude alias is replaced by the function (zsh)"
fi

# ------------------------------------------------------------ 4b. the proxy step

new_sandbox
S_PROXY=1
install_in_sandbox --yes --port "$PORT" || { cat "$S_ROOT/install.out" >&2; die "the install with the proxy failed"; }
assert_has "$S_ROOT/install.out" "Running tokred proxy enable"
assert_has "$S_DATA/tokred.toml" "127.0.0.1:$PORT"
assert_has "$S_DATA/tokred.toml" "enabled = true"
# Nobody answered the content question: it is not recorded.
assert_has "$S_DATA/tokred.toml" "log_bodies = false"
assert_has "$S_CLAUDE/settings.json" "http://127.0.0.1:$PORT"
assert_file "$S_CLAUDE/agents/tokred-dev.md"
in_sandbox "$(tokred_home)/bin/tokred" proxy status >"$S_ROOT/status.out" 2>&1 || true
assert_has "$S_ROOT/status.out" "tokred proxy: running"
in_sandbox "$(tokred_home)/bin/tokred" proxy stop >/dev/null 2>&1 || die "could not stop the proxy"
ok "the install runs tokred proxy enable: the port reaches [proxy] bind and the proxy runs"

new_sandbox
in_sandbox TOKRED_NO_PROXY=1 sh "$INSTALL_SH" --yes --port "$PORT" </dev/null >"$S_ROOT/install.out" 2>&1 || die "TOKRED_NO_PROXY install failed"
assert_has "$S_DATA/tokred.toml" "enabled = false"
assert_lacks "$S_DATA/tokred.toml" "127.0.0.1:$PORT"
assert_file "$S_CLAUDE/agents/tokred-dev.md"
ok "TOKRED_NO_PROXY=1 skips the proxy"

new_sandbox
install_in_sandbox --yes --no-configure || die "install --no-configure failed"
assert_lacks "$S_ROOT/install.out" "Running tokred proxy enable"
ok "--no-configure skips the proxy step too"

# A declined or failed `configure` stops the proxy step too. The release holds a fake tokred
# that logs its arguments and exits with $FAKE_CONFIGURE_EXIT for `configure`.
FAKE_REL="$WORK/fake-release"
mkdir -p "$FAKE_REL/pack/tokred-$TARGET"
cat >"$FAKE_REL/pack/tokred-$TARGET/tokred" <<'EOF'
#!/bin/sh
echo "$*" >>"$FAKE_LOG"
if [ "$1" = "configure" ]; then exit "${FAKE_CONFIGURE_EXIT:-0}"; fi
exit 0
EOF
chmod +x "$FAKE_REL/pack/tokred-$TARGET/tokred"
tar -czf "$FAKE_REL/$ASSET" -C "$FAKE_REL/pack" "tokred-$TARGET"
printf '%s  %s\n' "$(sha256_of "$FAKE_REL/$ASSET")" "$ASSET" >"$FAKE_REL/$ASSET.sha256"

for code in 0 1 2; do
    new_sandbox
    FAKE_LOG="$S_ROOT/fake.log"
    : >"$FAKE_LOG"
    in_sandbox TOKRED_RELEASE_BASE_URL="file://$FAKE_REL" FAKE_LOG="$FAKE_LOG" FAKE_CONFIGURE_EXIT="$code" \
        sh "$INSTALL_SH" --yes --port "$PORT" </dev/null >"$S_ROOT/install.out" 2>&1 || die "the install failed with configure exit $code"
    case "$code" in
        0)
            assert_has "$FAKE_LOG" "proxy enable --port $PORT --yes"
            assert_lacks "$S_ROOT/install.out" "did not finish"
            ;;
        1)
            assert_lacks "$FAKE_LOG" "proxy"
            assert_has "$S_ROOT/install.out" "'tokred configure' did not finish"
            ;;
        2)
            assert_lacks "$FAKE_LOG" "proxy"
            assert_lacks "$S_ROOT/install.out" "did not finish"
            assert_has "$S_ROOT/install.out" "configure was declined"
            ;;
    esac
done
ok "the proxy step runs only after a configure that finished: a declined (2) or failed one skips it"

# ------------------------------------------------------------ 5. the switches

new_sandbox
install_in_sandbox --yes --no-alias --port "$PORT" || die "install --no-alias failed"
assert_blocks "$S_HOME/.profile" 1
assert_has "$S_HOME/.profile" 'export PATH='
assert_lacks "$S_HOME/.profile" 'claude()'
ok "--no-alias keeps PATH and skips the wrapper"

new_sandbox
in_sandbox TOKRED_NO_ALIAS=1 sh "$INSTALL_SH" --yes --no-proxy --port "$PORT" </dev/null >/dev/null 2>&1 || die "TOKRED_NO_ALIAS install failed"
assert_lacks "$S_HOME/.profile" 'claude()'
ok "TOKRED_NO_ALIAS=1 skips the wrapper"

new_sandbox
install_in_sandbox --yes --no-modify-path --port "$PORT" || die "install --no-modify-path failed"
assert_blocks "$S_HOME/.profile" 1
assert_has "$S_HOME/.profile" 'claude()'
assert_lacks "$S_HOME/.profile" 'export PATH='
ok "--no-modify-path keeps the wrapper and skips PATH"

new_sandbox
install_in_sandbox --yes --no-modify-path --no-alias --no-configure || die "install with everything off failed"
assert_missing "$S_HOME/.profile"
assert_missing "$S_DATA"
assert_file "$(tokred_home)/bin/tokred"
ok "--no-modify-path --no-alias --no-configure only install the binary"

# ------------------------------------------------------------ 6. fish and zsh

new_sandbox
mkdir -p "$S_HOME/.config/fish/conf.d"
printf '# my fish setting\nset -gx EDITOR vim\n' >"$S_HOME/.config/fish/conf.d/tokred.fish"
S_SHELL=/bin/zsh
install_in_sandbox --yes --port "$PORT" || die "install with fish and zsh failed"
FISH_FILE="$S_HOME/.config/fish/conf.d/tokred.fish"
assert_has "$FISH_FILE" "set -gx EDITOR vim"
assert_has "$FISH_FILE" "function claude"
assert_has "$FISH_FILE" "    command claude \$argv"
assert_has "$FISH_FILE" "if not contains -- \"$S_TOKRED_HOME/bin\" \$PATH"
if command -v fish >/dev/null 2>&1; then fish -n "$FISH_FILE" || die "fish does not accept the generated file"; fi
assert_blocks "$S_HOME/.zshrc" 1
ok "fish gets its own block next to the user's lines; zsh gets a .zshrc even when it did not exist"

uninstall_in_sandbox || { cat "$S_ROOT/uninstall.out" >&2; die "the uninstall failed"; }
assert_file "$FISH_FILE"
assert_has "$FISH_FILE" "set -gx EDITOR vim"
assert_lacks "$FISH_FILE" "claude"
ok "uninstall keeps a fish file that holds other lines"

# ------------------------------------------------------------ 7. uninstall

new_sandbox
printf 'export FOO=1\n' >"$S_HOME/.bashrc"
printf '{"model": "opus"}\n' >"$S_CLAUDE/settings.json"
mkdir -p "$S_HOME/.config/fish"
install_in_sandbox --yes --port "$PORT" || die "the install before the uninstall failed"
assert_file "$S_HOME/.config/fish/conf.d/tokred.fish"
uninstall_in_sandbox || { cat "$S_ROOT/uninstall.out" >&2; die "the uninstall failed"; }

assert_missing "$S_TOKRED_HOME"
assert_lacks "$S_HOME/.bashrc" "tokred"
assert_has "$S_HOME/.bashrc" "export FOO=1"
assert_lacks "$S_HOME/.profile" "tokred"
assert_missing "$S_HOME/.config/fish/conf.d/tokred.fish"
assert_missing "$S_CLAUDE/agents/tokred-dev.md"
assert_lacks "$S_CLAUDE/settings.json" "tokred"
assert_missing "$S_DATA"
ok "uninstall removes everything without asking: the binary, the blocks, the hooks, the agents and the data folder"

new_sandbox
install_in_sandbox --yes --port "$PORT" || die "the install before the old options failed"
uninstall_in_sandbox --yes --purge || die "an uninstall with the old options failed"
assert_missing "$S_DATA"
assert_missing "$S_TOKRED_HOME"
ok "the old --yes and --purge options are accepted and change nothing"

new_sandbox
uninstall_in_sandbox || die "uninstalling something that was never installed failed"
ok "uninstalling without an installation is harmless"

# ------------------------------------------------------------ 8. failures leave nothing behind

new_sandbox
printf '0000000000000000000000000000000000000000000000000000000000000000  %s\n' "$ASSET" >"$WORK/bad.sha256"
mkdir -p "$WORK/tampered"
cp "$REL/$ASSET" "$WORK/tampered/$ASSET"
cp "$WORK/bad.sha256" "$WORK/tampered/$ASSET.sha256"
if in_sandbox TOKRED_RELEASE_BASE_URL="file://$WORK/tampered" sh "$INSTALL_SH" --yes </dev/null >"$S_ROOT/install.out" 2>&1; then
    die "a wrong checksum was accepted"
fi
assert_has "$S_ROOT/install.out" "checksum mismatch"
assert_missing "$S_TOKRED_HOME/bin/tokred"
assert_missing "$S_HOME/.profile"
ok "a wrong checksum stops the install before anything is written"

if in_sandbox TOKRED_RELEASE_BASE_URL="file://$WORK/nowhere" sh "$INSTALL_SH" --yes </dev/null >"$S_ROOT/install.out" 2>&1; then
    die "a missing release was accepted"
fi
assert_has "$S_ROOT/install.out" "could not download"
if in_sandbox TOKRED_RELEASE_BASE_URL="ftp://example.invalid" sh "$INSTALL_SH" --yes </dev/null >"$S_ROOT/install.out" 2>&1; then
    die "an ftp base address was accepted"
fi
assert_has "$S_ROOT/install.out" "only https:// and file:// are allowed"
for bad in abc 80 70000; do
    if install_in_sandbox --yes --port "$bad"; then die "port $bad was accepted"; fi
    assert_has "$S_ROOT/install.out" "between 1024 and 65535"
done
if install_in_sandbox --no-such-flag; then die "an unknown flag was accepted"; fi
ok "bad downloads, addresses, ports and flags fail with a clear message"

# ------------------------------------------------------------ 9. without --yes and without a terminal

if ! (: </dev/tty) 2>/dev/null; then
    new_sandbox
    install_in_sandbox --port "$PORT" || { cat "$S_ROOT/install.out" >&2; die "the install without --yes and without a terminal failed"; }
    assert_has "$S_ROOT/install.out" "no terminal to ask on"
    assert_file "$S_CLAUDE/agents/tokred-dev.md"
    ok "without a terminal the defaults are applied"

    new_sandbox
    S_PROXY=1
    install_in_sandbox --port "$PORT" || { cat "$S_ROOT/install.out" >&2; die "the install with the proxy, without --yes and without a terminal, failed"; }
    assert_has "$S_DATA/tokred.toml" "127.0.0.1:$PORT"
    assert_has "$S_DATA/tokred.toml" "enabled = true"
    assert_has "$S_DATA/tokred.toml" "log_bodies = false"
    in_sandbox "$(tokred_home)/bin/tokred" proxy stop >"$S_ROOT/stop.out" 2>&1 || { cat "$S_ROOT/stop.out" "$S_ROOT/install.out" >&2; die "could not stop the proxy"; }
    ok "without a terminal the proxy is enabled with the defaults and the given port"
fi

printf 'all installer tests passed\n'
