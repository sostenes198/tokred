#!/bin/sh
# tokred installer for macOS and Linux.
#
#   curl --proto '=https' --tlsv1.2 -LsSf https://raw.githubusercontent.com/sostenes198/tokred/main/install/install.sh | sh
#
# What it does, in order:
#   1. downloads tokred-<target>.tar.gz and its .sha256 from the GitHub release and checks the digest;
#   2. installs the binary into ${TOKRED_HOME:-$HOME/.tokred}/bin;
#   3. adds that folder to PATH and defines a `claude` wrapper in your shell profiles
#      (one marked block, so it can be replaced or removed cleanly);
#   4. runs `tokred configure`, which asks for the proxy port and whether conversation
#      content may be recorded for the dashboard.
# It never uses sudo and never writes outside TOKRED_HOME and your shell profile files.
#
# Options (environment variable in brackets):
#   -y, --yes            do not ask anything
#   --port N             proxy port [TOKRED_PORT]
#   --version X          install release vX instead of the latest [TOKRED_VERSION]
#   --no-modify-path     do not touch PATH [TOKRED_NO_MODIFY_PATH=1]
#   --no-alias           do not define the `claude` wrapper [TOKRED_NO_ALIAS=1]
#   --no-configure       only install the binary [TOKRED_NO_CONFIGURE=1]
#   -h, --help           this text
# Other environment variables: TOKRED_HOME, TOKRED_RELEASE_BASE_URL (an https:// or file://
# folder that holds the release assets; used by the tests and for mirrors).

set -eu

REPO="sostenes198/tokred"
BLOCK_BEGIN="# >>> tokred >>>"
BLOCK_END="# <<< tokred <<<"

say() { printf '%s\n' "$*"; }
step() { printf '==> %s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
fail() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

usage() {
    cat <<'EOF'
tokred installer (macOS and Linux)

Usage: install.sh [options]

  -y, --yes            do not ask anything
  --port N             proxy port (1024-65535)            [TOKRED_PORT]
  --version X          install release vX, not the latest [TOKRED_VERSION]
  --no-modify-path     do not touch PATH                  [TOKRED_NO_MODIFY_PATH=1]
  --no-alias           do not define the claude wrapper   [TOKRED_NO_ALIAS=1]
  --no-configure       only install the binary            [TOKRED_NO_CONFIGURE=1]
  -h, --help           this text

Also: TOKRED_HOME (default ~/.tokred), TOKRED_RELEASE_BASE_URL (an https:// or file://
folder with the release assets).
EOF
}

is_on() { [ -n "${1:-}" ] && [ "$1" != "0" ]; }

# ---------------------------------------------------------------- options

YES=0
PORT="${TOKRED_PORT:-}"
VERSION="${TOKRED_VERSION:-}"
NO_PATH=0
NO_ALIAS=0
NO_CONFIGURE=0
if is_on "${TOKRED_YES:-}"; then YES=1; fi
if is_on "${TOKRED_NO_MODIFY_PATH:-}"; then NO_PATH=1; fi
if is_on "${TOKRED_NO_ALIAS:-}"; then NO_ALIAS=1; fi
if is_on "${TOKRED_NO_CONFIGURE:-}"; then NO_CONFIGURE=1; fi

[ -n "${HOME:-}" ] || { printf 'error: HOME is not set\n' >&2; exit 1; }
TOKRED_HOME="${TOKRED_HOME:-$HOME/.tokred}"

while [ $# -gt 0 ]; do
    case "$1" in
        -y | --yes) YES=1 ;;
        --port)
            [ $# -ge 2 ] || fail "--port needs a number"
            PORT="$2"
            shift
            ;;
        --port=*) PORT="${1#--port=}" ;;
        --version)
            [ $# -ge 2 ] || fail "--version needs a version"
            VERSION="$2"
            shift
            ;;
        --version=*) VERSION="${1#--version=}" ;;
        --no-modify-path) NO_PATH=1 ;;
        --no-alias) NO_ALIAS=1 ;;
        --no-configure) NO_CONFIGURE=1 ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) fail "unknown option: $1 (try --help)" ;;
    esac
    shift
done

if [ -n "$PORT" ]; then
    case "$PORT" in
        *[!0-9]*) fail "the port must be a number between 1024 and 65535, not '$PORT'" ;;
    esac
    { [ "$PORT" -ge 1024 ] && [ "$PORT" -le 65535 ]; } 2>/dev/null ||
        fail "the port must be a number between 1024 and 65535, not '$PORT'"
fi

if [ "$(id -u)" = "0" ] && [ "$YES" != "1" ]; then
    fail "refusing to run as root (tokred installs for one user). Run it as your own user, or pass --yes to go on anyway."
fi

# ---------------------------------------------------------------- helpers

# Is there a terminal we can ask questions on, even when stdin is the pipe of `curl | sh`?
have_tty() { (: </dev/tty) 2>/dev/null; }

need() { command -v "$1" >/dev/null 2>&1 || fail "'$1' is required but was not found"; }

# download URL DEST: https:// with curl or wget, file:// by copying.
download() {
    case "$1" in
        file://*) cp "${1#file://}" "$2" || return 1 ;;
        https://*)
            if command -v curl >/dev/null 2>&1; then
                curl --proto '=https' --tlsv1.2 -fsSL -o "$2" "$1" || return 1
            elif command -v wget >/dev/null 2>&1; then
                wget -q -O "$2" "$1" || return 1
            else
                fail "curl or wget is required to download tokred"
            fi
            ;;
        *) fail "unsupported download address: $1 (only https:// and file:// are allowed)" ;;
    esac
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        fail "sha256sum or shasum is required to verify the download"
    fi
}

detect_target() {
    os=$(uname -s)
    arch=$(uname -m)
    case "$arch" in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) fail "unsupported CPU architecture: $arch" ;;
    esac
    case "$os" in
        Darwin)
            # A shell started under Rosetta reports x86_64 on an Apple Silicon Mac.
            if [ "$arch" = "x86_64" ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || true)" = "1" ]; then
                arch=aarch64
            fi
            printf '%s-apple-darwin' "$arch"
            ;;
        Linux) printf '%s-unknown-linux-musl' "$arch" ;;
        *) fail "unsupported operating system: $os (Windows uses install.ps1)" ;;
    esac
}

# profile_text BIN_DISPLAY: the block for POSIX shells.
posix_block() {
    printf '%s\n' "$BLOCK_BEGIN"
    printf '%s\n' "# Managed by the tokred installer. Remove it with the tokred uninstall script."
    if [ "$NO_PATH" != "1" ]; then
        printf 'case ":$PATH:" in *":%s:"*) ;; *) export PATH="%s:$PATH" ;; esac\n' "$1" "$1"
    fi
    if [ "$NO_ALIAS" != "1" ]; then
        printf '%s\n' 'unalias claude 2>/dev/null'
        printf '%s\n' 'claude() { if command -v tokred >/dev/null 2>&1; then tokred initialize; fi; command claude "$@"; }'
    fi
    printf '%s\n' "$BLOCK_END"
}

fish_block() {
    printf '%s\n' "$BLOCK_BEGIN"
    printf '%s\n' "# Managed by the tokred installer. Remove it with the tokred uninstall script."
    if [ "$NO_PATH" != "1" ]; then
        printf 'if not contains -- "%s" $PATH; set -gx PATH "%s" $PATH; end\n' "$1" "$1"
    fi
    if [ "$NO_ALIAS" != "1" ]; then
        printf '%s\n' 'function claude'
        printf '%s\n' '    if type -q tokred'
        printf '%s\n' '        tokred initialize'
        printf '%s\n' '    end'
        printf '%s\n' '    command claude $argv'
        printf '%s\n' 'end'
    fi
    printf '%s\n' "$BLOCK_END"
}

# put_block FILE BLOCKFILE: replaces our block in FILE (or appends it). The file is rewritten
# through `cat >` so a symlinked dotfile stays a symlink and keeps its permissions.
put_block() {
    file=$1
    block=$2
    mkdir -p "$(dirname "$file")"
    [ -e "$file" ] || : >"$file"
    stripped="$TMP/profile.stripped"
    awk -v b="$BLOCK_BEGIN" -v e="$BLOCK_END" '
        $0 == b { skip = 1; next }
        $0 == e { skip = 0; next }
        !skip { print }
    ' "$file" >"$stripped"
    # A last line without a newline would swallow our marker.
    if [ -s "$stripped" ] && [ "$(tail -c 1 "$stripped" | wc -l | tr -d ' ')" = "0" ]; then
        printf '\n' >>"$stripped"
    fi
    cat "$stripped" "$block" >"$file"
    say "    updated $file"
}

# ---------------------------------------------------------------- install

TMP=$(mktemp -d 2>/dev/null || mktemp -d -t tokred) || fail "could not create a temporary folder"
trap 'rm -rf "$TMP"' EXIT
trap 'exit 1' INT TERM HUP

need uname
need tar
need awk
need mktemp

TARGET=$(detect_target)
ASSET="tokred-$TARGET.tar.gz"

if [ -n "${TOKRED_RELEASE_BASE_URL:-}" ]; then
    BASE="${TOKRED_RELEASE_BASE_URL%/}"
elif [ -n "$VERSION" ]; then
    BASE="https://github.com/$REPO/releases/download/v${VERSION#v}"
else
    BASE="https://github.com/$REPO/releases/latest/download"
fi

BIN_DIR="$TOKRED_HOME/bin"

step "Installing tokred for $TARGET into $BIN_DIR"
step "Downloading $ASSET"
download "$BASE/$ASSET" "$TMP/$ASSET" || fail "could not download $BASE/$ASSET (is there a release for this platform?)"
download "$BASE/$ASSET.sha256" "$TMP/$ASSET.sha256" || fail "could not download $BASE/$ASSET.sha256"

step "Verifying the checksum"
expected=$(awk 'NR == 1 { print $1 }' "$TMP/$ASSET.sha256" | tr 'A-F' 'a-f' | tr -d '*')
actual=$(sha256_of "$TMP/$ASSET" | tr 'A-F' 'a-f')
[ -n "$expected" ] || fail "the checksum file is empty"
[ "$expected" = "$actual" ] || fail "checksum mismatch (expected $expected, got $actual); the download was discarded"

step "Unpacking"
mkdir -p "$TMP/unpacked"
tar -xzf "$TMP/$ASSET" -C "$TMP/unpacked" || fail "could not unpack $ASSET"
FOUND=$(find "$TMP/unpacked" -type f -name tokred | awk 'NR == 1 { print }')
[ -n "$FOUND" ] || fail "the archive does not contain a tokred binary"

mkdir -p "$BIN_DIR" || fail "could not create $BIN_DIR"
cp "$FOUND" "$BIN_DIR/tokred.new" || fail "could not write into $BIN_DIR"
chmod 755 "$BIN_DIR/tokred.new"
mv -f "$BIN_DIR/tokred.new" "$BIN_DIR/tokred" || fail "could not put the binary in place"
INSTALLED=$("$BIN_DIR/tokred" --version 2>/dev/null) || fail "the installed binary does not run on this machine"
say "    installed $INSTALLED at $BIN_DIR/tokred"

# How the block names the folder: keep $HOME symbolic so the profile survives a moved home.
case "$BIN_DIR" in
    "$HOME"/*) BIN_TEXT="\$HOME${BIN_DIR#"$HOME"}" ;;
    *) BIN_TEXT="$BIN_DIR" ;;
esac
case "$BIN_DIR" in
    *'"'* | *'`'* | *'\'*) fail "TOKRED_HOME may not contain quotes, backticks or backslashes" ;;
esac

if [ "$NO_PATH" = "1" ] && [ "$NO_ALIAS" = "1" ]; then
    step "Leaving your shell profiles alone (--no-modify-path and --no-alias)"
else
    step "Updating your shell profiles"
    posix_block "$BIN_TEXT" >"$TMP/block.sh"
    fish_block "$BIN_TEXT" >"$TMP/block.fish"
    ZSH_DIR="${ZDOTDIR:-$HOME}"
    put_block "$HOME/.profile" "$TMP/block.sh"
    for f in "$HOME/.bashrc" "$HOME/.bash_profile" "$ZSH_DIR/.zshrc"; do
        if [ -e "$f" ]; then
            put_block "$f" "$TMP/block.sh"
        fi
    done
    # Zsh is the default macOS shell and may have no .zshrc yet.
    case "${SHELL:-}" in
        */zsh)
            if [ ! -e "$ZSH_DIR/.zshrc" ]; then
                put_block "$ZSH_DIR/.zshrc" "$TMP/block.sh"
            fi
            ;;
    esac
    if command -v fish >/dev/null 2>&1 || [ -d "$HOME/.config/fish" ]; then
        put_block "$HOME/.config/fish/conf.d/tokred.fish" "$TMP/block.fish"
    fi
fi

CONFIGURED=0
if [ "$NO_CONFIGURE" = "1" ]; then
    step "Skipping tokred configure (--no-configure); run it yourself when you are ready"
else
    step "Running tokred configure"
    set -- configure
    if [ -n "$PORT" ]; then
        set -- "$@" --proxy-port "$PORT"
    fi
    # `curl | sh` leaves stdin as the pipe: talk to the terminal when there is one.
    if [ "$YES" = "1" ]; then
        set -- "$@" --yes
        if PATH="$BIN_DIR:$PATH" "$BIN_DIR/tokred" "$@" </dev/null; then CONFIGURED=1; fi
    elif [ -t 0 ]; then
        if PATH="$BIN_DIR:$PATH" "$BIN_DIR/tokred" "$@"; then CONFIGURED=1; fi
    elif have_tty; then
        if PATH="$BIN_DIR:$PATH" "$BIN_DIR/tokred" "$@" </dev/tty; then CONFIGURED=1; fi
    else
        say "    no terminal to ask on: applying the defaults (--yes)"
        set -- "$@" --yes
        if PATH="$BIN_DIR:$PATH" "$BIN_DIR/tokred" "$@" </dev/null; then CONFIGURED=1; fi
    fi
    if [ "$CONFIGURED" != "1" ]; then
        warn "tokred is installed, but 'tokred configure' did not finish (is Claude Code installed?). Run it again when you are ready."
    fi
fi

say ""
say "tokred is installed."
if [ "$NO_PATH" != "1" ]; then
    say "  Open a new terminal, or run:  export PATH=\"$BIN_DIR:\$PATH\""
else
    say "  PATH was not changed; run tokred as $BIN_DIR/tokred."
fi
if [ "$NO_ALIAS" != "1" ]; then
    say "  New shells have claude wrapped: it runs 'tokred initialize' first. 'tokred proxy disable' / 'tokred proxy enable' is the switch; 'command claude' always runs the real binary."
fi
say "  Check it with: tokred doctor      Remove it with the uninstall script (see the README)."
exit 0
