#!/bin/sh
# tokred uninstaller for macOS and Linux.
#
#   curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/sostenes198/tokred/main/install/uninstall.sh | sh
#
# It runs `tokred uninstall` (which stops the proxy, removes its user service and takes the
# hooks and agents out of Claude Code), then deletes the binary, the TOKRED_HOME folder when it
# is empty, and the PATH entry and `claude` wrapper from your shell profiles.
#
# Options: -y/--yes (do not ask; keeps your data unless --purge), --purge (also delete tokred's
# data folder), -h/--help. TOKRED_HOME defaults to ~/.tokred.

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

Usage: uninstall.sh [options]

  -y, --yes     do not ask anything (keeps tokred's data folder unless --purge)
  --purge       also delete tokred's data folder (database, config, logs, reports)
  -h, --help    this text

TOKRED_HOME (default ~/.tokred) is where the install script put the binary.
EOF
}

YES=0
PURGE=0
case "${TOKRED_YES:-}" in "" | 0) ;; *) YES=1 ;; esac
case "${TOKRED_PURGE:-}" in "" | 0) ;; *) PURGE=1 ;; esac
[ -n "${HOME:-}" ] || { printf 'error: HOME is not set\n' >&2; exit 1; }
TOKRED_HOME="${TOKRED_HOME:-$HOME/.tokred}"

while [ $# -gt 0 ]; do
    case "$1" in
        -y | --yes) YES=1 ;;
        --purge) PURGE=1 ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) fail "unknown option: $1 (try --help)" ;;
    esac
    shift
done

if [ "$(id -u)" = "0" ] && [ "$YES" != "1" ]; then
    fail "refusing to run as root (tokred is installed per user). Run it as your own user, or pass --yes."
fi

have_tty() { (: </dev/tty) 2>/dev/null; }

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

BIN_DIR="$TOKRED_HOME/bin"
EXE="$BIN_DIR/tokred"

if [ -x "$EXE" ]; then
    step "Running tokred uninstall"
    set -- uninstall
    if [ "$PURGE" = "1" ]; then
        set -- "$@" --purge
    fi
    # Same terminal rules as the installer: `curl | sh` leaves stdin as the pipe.
    rc=0
    if [ "$YES" = "1" ]; then
        set -- "$@" --yes
        "$EXE" "$@" </dev/null || rc=$?
    elif [ -t 0 ]; then
        "$EXE" "$@" || rc=$?
    elif have_tty; then
        "$EXE" "$@" </dev/tty || rc=$?
    else
        say "    no terminal to ask on: continuing without confirmation (--yes)"
        set -- "$@" --yes
        "$EXE" "$@" </dev/null || rc=$?
    fi
    case "$rc" in
        0) ;;
        # `tokred uninstall` exits with 2 when you answer "no".
        2)
            say "Nothing was changed."
            exit 0
            ;;
        *) fail "'tokred uninstall' failed; the binary was kept so Claude Code does not point at nothing. Fix the problem above and run this again." ;;
    esac
else
    step "No tokred binary at $EXE: skipping 'tokred uninstall'"
fi

step "Removing the binary"
rm -f "$EXE" "$BIN_DIR/tokred.new" "$BIN_DIR/tokred.old"
rmdir "$BIN_DIR" 2>/dev/null || true
rmdir "$TOKRED_HOME" 2>/dev/null || true
if [ -e "$TOKRED_HOME" ]; then
    say "    kept $TOKRED_HOME (it is not empty)"
else
    say "    removed $TOKRED_HOME"
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
