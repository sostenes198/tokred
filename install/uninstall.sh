#!/bin/sh
# tokred uninstaller for macOS and Linux.
#
#   curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/sostenes198/tokred/main/install/uninstall.sh | sh
#
# It removes everything tokred put on the machine, without asking: when the binary is there it
# runs `tokred uninstall` (stops the proxy, removes its user service, takes the hooks, agents and
# proxy URL out of Claude Code and deletes tokred's data folder), and `tokred uninstall` then
# runs this same script again with --cleanup-only for the last part: the binary, its folder, the
# PATH entry and the `claude` wrapper in your shell profiles. Without the binary this script does
# that last part by itself.
#
# Options: -h/--help. Internal (used by `tokred uninstall`): --cleanup-only, --home DIR,
# --wait-pid PID. TOKRED_HOME defaults to ~/.tokred.

set -eu

BLOCK_BEGIN="# >>> tokred >>>"
BLOCK_END="# <<< tokred <<<"

say() { printf '%s\n' "$*"; }
step() { printf '==> %s\n' "$*"; }
fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
tokred uninstaller (macOS and Linux)

Usage: uninstall.sh [-h]

Removes tokred completely, without asking: its proxy and user service, its hooks, agents and
proxy URL in Claude Code, its data folder, the binary, the PATH entry and the `claude` wrapper.

TOKRED_HOME (default ~/.tokred) is where the install script put the binary.
EOF
}

CLEANUP_ONLY=0
WAIT_PID=
HOME_DIR=
[ -n "${HOME:-}" ] || { printf 'error: HOME is not set\n' >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        # Old options, kept so existing commands do not fail: nothing is asked any more.
        -y | --yes | --purge) ;;
        --cleanup-only) CLEANUP_ONLY=1 ;;
        --home)
            [ $# -ge 2 ] || fail "--home needs a folder"
            HOME_DIR=$2
            shift
            ;;
        --wait-pid)
            [ $# -ge 2 ] || fail "--wait-pid needs a process id"
            WAIT_PID=$2
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) fail "unknown option: $1 (try --help)" ;;
    esac
    shift
done

# Run by hand there is no `tokred uninstall` yet: the folder is TOKRED_HOME or ~/.tokred.
if [ "$CLEANUP_ONLY" = "0" ]; then
    HOME_DIR="${TOKRED_HOME:-$HOME/.tokred}"
    EXE="$HOME_DIR/bin/tokred"
    if [ -x "$EXE" ]; then
        step "Running tokred uninstall"
        exec "$EXE" uninstall
    fi
    step "No tokred binary at $EXE: removing what is left in your shell"
fi

# strip_block FILE: removes our block; a file that ends up empty is deleted when DELETE_EMPTY=1.
strip_block() {
    file=$1
    [ -e "$file" ] || return 0
    grep -qF "$BLOCK_BEGIN" "$file" 2>/dev/null || return 0
    stripped="$TMP/profile.stripped"
    awk -v b="$BLOCK_BEGIN" -v e="$BLOCK_END" '
        $0 == b { skip = 1; next }
        $0 == e { skip = 0; next }
        !skip { print }
    ' "$file" >"$stripped"
    if [ "${DELETE_EMPTY:-0}" = "1" ] && ! grep -q '[^[:space:]]' "$stripped"; then
        rm -f "$file"
        say "    removed $file"
    else
        cat "$stripped" >"$file"
        say "    cleaned $file"
    fi
}

TMP=$(mktemp -d 2>/dev/null || mktemp -d -t tokred) || fail "could not create a temporary folder"
trap 'rm -rf "$TMP"' EXIT
trap 'exit 1' INT TERM HUP

# `tokred uninstall` starts this script and exits: wait for it, then the binary can go.
if [ -n "$WAIT_PID" ]; then
    waited=0
    while kill -0 "$WAIT_PID" 2>/dev/null && [ "$waited" -lt 100 ]; do
        sleep 0.3
        waited=$((waited + 1))
    done
fi

if [ -n "$HOME_DIR" ]; then
    step "Removing the binary"
    BIN_DIR="$HOME_DIR/bin"
    rm -f "$BIN_DIR/tokred" "$BIN_DIR/tokred.new" "$BIN_DIR/tokred.old"
    rmdir "$BIN_DIR" 2>/dev/null || true
    rmdir "$HOME_DIR" 2>/dev/null || true
    if [ -e "$HOME_DIR" ]; then
        say "    kept $HOME_DIR (it is not empty)"
    else
        say "    removed $HOME_DIR"
    fi
fi

step "Cleaning your shell profiles"
ZSH_DIR="${ZDOTDIR:-$HOME}"
for f in "$HOME/.profile" "$HOME/.bashrc" "$HOME/.bash_profile" "$ZSH_DIR/.zshrc"; do
    strip_block "$f"
done
DELETE_EMPTY=1 strip_block "$HOME/.config/fish/conf.d/tokred.fish"

say ""
say "tokred is removed. Open a new terminal so the 'claude' wrapper and the PATH entry are gone."
exit 0
