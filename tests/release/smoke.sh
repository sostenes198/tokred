#!/bin/sh
# Release smoke test for macOS and Linux: installs tokred from a PUBLISHED GitHub release and checks,
# with assertions on real state, every feature the product ships. POSIX sh (dash, ash/busybox, bash,
# macOS sh); the JSON and database checks are in check.py, the fake Anthropic API in fake_upstream.py.
#
#   sh tests/release/smoke.sh v0.1.0-rc.1
#
# It downloads the release's own install.sh and uninstall.sh (and, through install.sh, the binary),
# so what it proves is what a user gets. It exits 0 only when every assertion held; on the first
# failed one it prints `not ok - <what>` (and the last 30 lines of every log) and exits 1.
#
# Needs: curl, tar, awk, sed, grep, python3 (3.7+; the sqlite3 module or the sqlite3 tool), network.
# Everything runs in a throwaway directory: HOME, TOKRED_HOME, TOKRED_DATA_DIR and CLAUDE_CONFIG_DIR
# point into it, the fake upstream is the only "Anthropic" the proxy talks to, nothing real is touched.
#
# Arguments / environment knobs:
#   $1 | SMOKE_TAG       release tag to test, e.g. v0.1.0-rc.1 (required)
#   SMOKE_REPO           GitHub repository (default sostenes198/tokred). The release assets are read from
#                        it; note the downloaded install.sh fetches the binary from the repository it
#                        was built for, and the update step asks this repository's "latest" API.
#   SMOKE_PORT           proxy port (default: a free one chosen at random)
#   SMOKE_DIR            working directory (default: a new mktemp -d); removed at the end unless
#                        SMOKE_KEEP=1 (a given SMOKE_DIR is never removed unless it was created here)
#   SMOKE_KEEP=1         keep the working directory (sandbox, logs, downloads) after the run
#   SMOKE_LOG_DIR        on failure the logs are copied here (CI uploads them as an artifact)
#   SMOKE_CONTINUE=1     do not stop at the first failed assertion: count it, go on, exit 1 at the end
#                        (a diagnostic mode: a failure can cascade into later ones)
#   SMOKE_SKIP_UPDATE=1  skip step 6 (`tokred update`, the only step that needs the GitHub API)
#   SMOKE_PYTHON         the Python 3 to use (default: python3 on PATH)
#   GITHUB_TOKEN         optional: used only by this script's own probe of the GitHub API, to avoid
#                        the anonymous rate limit (tokred itself is always anonymous)

set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
CHECK="$HERE/check.py"
FAKE="$HERE/fake_upstream.py"

TAG="${1:-${SMOKE_TAG:-}}"
REPO="${SMOKE_REPO:-sostenes198/tokred}"
case "$TAG" in
    v[0-9]*) ;;
    *)
        printf 'usage: sh tests/release/smoke.sh <tag>   (for example v0.1.0-rc.1)\n' >&2
        exit 2
        ;;
esac
VERSION="${TAG#v}"
BEGIN="# >>> tokred >>>"

# ------------------------------------------------------------------ assertions

ASSERTS=0
FAILS=0
SKIPS=0
STEP="start"

step() {
    STEP="$*"
    printf '\n==> %s\n' "$*"
}
ok() {
    ASSERTS=$((ASSERTS + 1))
    printf 'ok - %s\n' "$*"
}
note() { printf 'note: %s\n' "$*"; }
skip() {
    SKIPS=$((SKIPS + 1))
    printf 'skip - %s\n' "$*"
}
die() {
    printf 'not ok - [%s] %s\n' "$STEP" "$*" >&2
    FAILS=$((FAILS + 1))
    if [ "${SMOKE_CONTINUE:-0}" = "1" ]; then
        return 0
    fi
    exit 1
}

assert_file() { if [ -f "$1" ]; then ok "$2"; else die "$2: missing file $1"; fi; }
assert_missing() { if [ ! -e "$1" ]; then ok "$2"; else die "$2: should not exist: $1"; fi; }
assert_has() { if grep -qF -- "$2" "$1" 2>/dev/null; then ok "$3"; else die "$3: $1 does not contain: $2"; fi; }
assert_eq() { if [ "$1" = "$2" ]; then ok "$3"; else die "$3: got '$1', expected '$2'"; fi; }
assert_same_file() {
    if cmp -s "$1" "$2"; then ok "$3"; else
        die "$3: $1 differs from $2:
$(diff "$2" "$1" 2>&1 | head -20)"
    fi
}

# chk ARGS...: runs check.py (which prints one `ok - ...` line per assertion it makes).
chk() {
    CHK_RC=0
    CHK_OUT=$("$PY" -I "$CHECK" "$@" 2>&1) || CHK_RC=$?
    if [ "$CHK_RC" -ne 0 ]; then
        printf '%s\n' "$CHK_OUT" >&2
        die "check.py $1 failed (see above)"
        return 0
    fi
    printf '%s\n' "$CHK_OUT"
    n=$(printf '%s\n' "$CHK_OUT" | grep -c '^ok - ' || true)
    ASSERTS=$((ASSERTS + n))
}

# ------------------------------------------------------------------ the sandbox

command -v curl >/dev/null 2>&1 || { printf 'error: curl is required\n' >&2; exit 2; }
PY_CMD="${SMOKE_PYTHON:-python3}"
PY=$(command -v "$PY_CMD" 2>/dev/null) || { printf 'error: %s is required\n' "$PY_CMD" >&2; exit 2; }
# The absolute interpreter (a pyenv shim would not survive `env -i` and a new HOME).
PY=$("$PY" -c 'import sys; print(sys.executable)') || exit 2
"$PY" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 7) else 1)' || { printf 'error: Python 3.7+ is required\n' >&2; exit 2; }

if [ -n "${SMOKE_DIR:-}" ]; then
    W="$SMOKE_DIR"
    mkdir -p "$W"
    CREATED_W=0
else
    W=$(mktemp -d 2>/dev/null || mktemp -d -t tokredsmoke)
    CREATED_W=1
fi
W=$(cd "$W" && pwd)

HOMEDIR="$W/home"
TH="$W/th"
DATA="$W/data"
CLAUDE="$W/claude"
DL="$W/dl"
OUT="$W/out"
ORIG="$W/orig"
FAKEBIN="$W/fakebin"
JAR="$W/cookies"
TOKRED="$TH/bin/tokred"
BASEPATH="$PATH"
mkdir -p "$HOMEDIR" "$CLAUDE" "$DL" "$OUT" "$ORIG" "$FAKEBIN"

pick_port() {
    "$PY" -I -c 'import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()'
}
PORT="${SMOKE_PORT:-$(pick_port)}"
RELEASES_API="https://api.github.com/repos/$REPO/releases/latest"
BASE="https://github.com/$REPO/releases/download/$TAG"

# sb COMMAND ARGS...: COMMAND with only the sandbox variables visible (SBPATH overrides PATH, EXTRA_ENV
# adds NAME=value words). TOKRED_RELEASE_BASE_URL is never set: the installer downloads from GitHub.
EXTRA_ENV="TOKRED_OFFLINE=1"
sb() {
    # shellcheck disable=SC2086 # EXTRA_ENV is a list of NAME=value words without spaces
    env -i PATH="${SBPATH:-$TH/bin:$BASEPATH}" HOME="$HOMEDIR" TOKRED_HOME="$TH" TOKRED_DATA_DIR="$DATA" \
        CLAUDE_CONFIG_DIR="$CLAUDE" LANG=C TERM=dumb NO_COLOR=1 $EXTRA_ENV "$@"
}

# tk NAME ARGS...: runs the installed tokred, keeps stdout+stderr in out/NAME.out, sets RC.
RC=0
tk() {
    name=$1
    shift
    RC=0
    sb "$TOKRED" "$@" >"$OUT/$name.out" 2>&1 </dev/null || RC=$?
}
rc_is() { if [ "$RC" = "$1" ]; then ok "$2"; else die "$2: exit status $RC, expected $1 ($(head -c 400 "$OUT/$3.out" 2>/dev/null))"; fi; }

http_code() { curl -s -o /dev/null --max-time 10 -w '%{http_code}' "$@" 2>/dev/null || true; }
health_code() { http_code "http://127.0.0.1:$PORT/__tokred/health"; }
wait_health() {
    i=0
    while [ "$i" -lt 60 ]; do
        [ "$(health_code)" = "200" ] && return 0
        i=$((i + 1))
        sleep 0.3
    done
    return 1
}
wait_down() {
    i=0
    while [ "$i" -lt 60 ]; do
        [ "$(health_code)" = "000" ] && return 0
        i=$((i + 1))
        sleep 0.3
    done
    return 1
}

# ps_lines: every process, one per line, with its command line (procps, then busybox spellings).
ps_lines() {
    ps -axo pid,command 2>/dev/null || ps -eo pid,args 2>/dev/null || ps 2>/dev/null || true
}

# ------------------------------------------------------------------ cleanup (always)

FAKE_PID=""
# shellcheck disable=SC2329 # run by the EXIT trap
cleanup() {
    status=$?
    trap - EXIT INT TERM HUP
    if [ "$status" -eq 0 ] && [ "$FAILS" -gt 0 ]; then status=1; fi
    if [ "$status" -ne 0 ]; then
        printf '\n---- tail of every log (last 30 lines) ----\n' >&2
        for f in "$W"/*.log "$OUT"/*.out "$DATA"/logs/*; do
            [ -f "$f" ] || continue
            printf '\n--- %s\n' "$f" >&2
            tail -n 30 "$f" >&2 || true
        done
        if [ -n "${SMOKE_LOG_DIR:-}" ]; then
            mkdir -p "$SMOKE_LOG_DIR" 2>/dev/null || true
            cp "$W"/*.log "$OUT"/*.out "$SMOKE_LOG_DIR"/ 2>/dev/null || true
            cp "$DATA"/logs/* "$SMOKE_LOG_DIR"/ 2>/dev/null || true
        fi
    fi
    # The proxy of the sandbox, by its binary when it is still there, else by its pid file (checked).
    if [ -x "$TOKRED" ]; then
        sb "$TOKRED" proxy stop >/dev/null 2>&1 </dev/null || true
    fi
    if [ -f "$DATA/proxy/pid" ]; then
        pid=$(awk 'NR == 1 { print $1 }' "$DATA/proxy/pid" 2>/dev/null || true)
        case "$pid" in
            '' | *[!0-9]*) ;;
            *)
                if ps_lines | grep -F "$W/" | awk -v p="$pid" '$1 == p { found = 1 } END { exit !found }'; then
                    kill "$pid" 2>/dev/null || true
                fi
                ;;
        esac
    fi
    if [ -n "$FAKE_PID" ]; then
        kill "$FAKE_PID" 2>/dev/null || true
    fi
    if [ "${SMOKE_KEEP:-0}" = "1" ]; then
        printf 'kept: %s\n' "$W" >&2
    elif [ "$CREATED_W" = "1" ]; then
        rm -rf "$W"
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP

# ------------------------------------------------------------------ the user's starting state

# The settings.json tokred must not damage, and shell profiles with the user's own lines.
cat >"$ORIG/settings.json" <<'EOF'
{"model":"opus","hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo user-hook"}]}]},"env":{"FOO":"bar"}}
EOF
cp "$ORIG/settings.json" "$CLAUDE/settings.json"
printf '# my profile\nexport USER_PROFILE_LINE=1\nalias ll="ls -l"\n' >"$HOMEDIR/.profile"
printf '# my bashrc\nexport USER_BASHRC_LINE=1\n' >"$HOMEDIR/.bashrc"
printf '# my zshrc\nexport USER_ZSHRC_LINE=1\n' >"$HOMEDIR/.zshrc"
for f in .profile .bashrc .zshrc; do cp "$HOMEDIR/$f" "$ORIG/$f"; done
(cd "$HOMEDIR" && find . | sort) >"$ORIG/home.list"
(cd "$CLAUDE" && find . | sort) >"$ORIG/claude.list"

strip_block() { awk -v b="$BEGIN" -v e="# <<< tokred <<<" '$0 == b { s = 1; next } $0 == e { s = 0; next } !s { print }' "$1"; }
blocks_in() { grep -cF -- "$BEGIN" "$1" || true; }

# ------------------------------------------------------------------ 0. the release assets

step "0. download the release assets of $TAG from $REPO"
for asset in install.sh uninstall.sh; do
    curl --proto '=https' --tlsv1.2 -fsSL --retry 3 -o "$DL/$asset" "$BASE/$asset" ||
        die "could not download $BASE/$asset (is $TAG published in $REPO?)"
    [ -s "$DL/$asset" ] || die "$asset of $TAG is empty"
    ok "the release has $asset"
done
head -n 1 "$DL/install.sh" | grep -q '^#!/bin/sh' || die "install.sh of $TAG does not start with #!/bin/sh"
ok "install.sh is a POSIX shell script"

# ------------------------------------------------------------------ 1. install

step "1. install from the release: install.sh --yes --version $VERSION --port $PORT"
RC=0
sb sh "$DL/install.sh" --yes --version "$VERSION" --port "$PORT" >"$OUT/install.out" 2>&1 </dev/null || RC=$?
rc_is 0 "install.sh succeeded" install
assert_has "$OUT/install.out" "Verifying the checksum" "the installer downloaded the archive and verified its checksum"
assert_has "$OUT/install.out" "tokred is installed." "the installer said it is installed"

if [ -x "$TOKRED" ]; then ok "the binary is in \$TOKRED_HOME/bin and executable"; else die "no executable binary at $TOKRED"; fi
assert_eq "$(sb "$TOKRED" --version </dev/null)" "tokred $VERSION" "tokred --version equals the tag ($TAG)"

for f in .profile .bashrc .zshrc; do
    assert_eq "$(blocks_in "$HOMEDIR/$f")" "1" "$f has exactly one tokred block"
    strip_block "$HOMEDIR/$f" >"$W/stripped"
    assert_same_file "$W/stripped" "$ORIG/$f" "$f: the user's own lines are intact"
done
assert_has "$HOMEDIR/.profile" "export PATH=\"$TH/bin:\$PATH\"" "the block puts \$TOKRED_HOME/bin on PATH"
assert_has "$HOMEDIR/.profile" 'claude() { if command -v tokred >/dev/null 2>&1; then tokred initialize; fi; command claude "$@"; }' "the block defines the claude function"

chk settings "$CLAUDE/settings.json" --hooks present --bin "$TOKRED" --user --url "http://127.0.0.1:$PORT"
chk agents "$CLAUDE/agents" --count 6
chk toml "$DATA/tokred.toml" proxy enabled true
chk toml "$DATA/tokred.toml" proxy bind "\"127.0.0.1:$PORT\""
chk toml "$DATA/tokred.toml" proxy log_bodies false
assert_eq "$(health_code)" "200" "the proxy answers GET /__tokred/health on port $PORT"
curl -s --max-time 10 "http://127.0.0.1:$PORT/__tokred/health" >"$OUT/health.json"
assert_has "$OUT/health.json" "\"version\":\"$VERSION\"" "the proxy that answers is version $VERSION"
chk marker "$DATA/tokred.db" --is "$VERSION"

# ------------------------------------------------------------------ 2. the claude wrapper

step "2. the claude wrapper"
cat >"$FAKEBIN/claude" <<'EOF'
#!/bin/sh
# The fake claude: prints each argument in brackets, one per line.
for a in "$@"; do printf '[%s]\n' "$a"; done
EOF
chmod 755 "$FAKEBIN/claude"
cat >"$W/call.inc" <<'EOF'
claude 'a b' "it's" '' '$HOME' '*' 'say "hi"' '--flag=x y'
EOF
cat >"$W/call-notokred.inc" <<'EOF'
PATH="$FAKEBIN:/usr/bin:/bin"
export PATH
if command -v tokred >/dev/null 2>&1; then printf 'TOKRED-IS-STILL-ON-PATH\n'; fi
claude 'a b' "it's" '' '$HOME' '*' 'say "hi"' '--flag=x y'
EOF
cat >"$W/type.inc" <<'EOF'
type claude
EOF
cat >"$W/expected-args.txt" <<'EOF'
[a b]
[it's]
[]
[$HOME]
[*]
[say "hi"]
[--flag=x y]
EOF

# wrapper_run SHELL PROFILE INC OUTFILE: a fresh shell that sources the profile, then runs INC.
wrapper_run() {
    SBPATH="$FAKEBIN:$TH/bin:$BASEPATH" sb env FAKEBIN="$FAKEBIN" "$1" -c '. "$1"; . "$2"' wrapper "$HOMEDIR/$2" "$3" >"$4" 2>&1 </dev/null
}

# What `initialize` does must be visible: stop the proxy and take the URL out first.
tk proxy-stop proxy stop
rc_is 0 "tokred proxy stop succeeded" proxy-stop
if wait_down; then ok "the proxy is down before the wrapper runs"; else die "the proxy still answers after proxy stop"; fi
chk edit "$CLAUDE/settings.json" --drop-url
chk settings "$CLAUDE/settings.json" --no-url

wrapper_run sh .profile "$W/type.inc" "$OUT/wrapper-type.out" || true
if grep -qi 'function' "$OUT/wrapper-type.out"; then ok "claude is a shell function in a fresh sh after sourcing the profile"; else die "claude is not a function: $(cat "$OUT/wrapper-type.out")"; fi
wrapper_run sh .profile "$W/call.inc" "$OUT/wrapper-sh.out" || die "the claude wrapper failed in sh: $(cat "$OUT/wrapper-sh.out")"
assert_same_file "$OUT/wrapper-sh.out" "$W/expected-args.txt" "sh: arguments with spaces, quotes, \$HOME, an empty string and * reach claude intact"
assert_eq "$(health_code)" "200" "tokred initialize ran: the stopped proxy is running again"
chk settings "$CLAUDE/settings.json" --url "http://127.0.0.1:$PORT"

for pair in "bash:.bashrc" "dash:.profile" "zsh:.zshrc"; do
    shell=${pair%%:*}
    profile=${pair#*:}
    if ! command -v "$shell" >/dev/null 2>&1; then
        skip "$shell is not installed here"
        continue
    fi
    wrapper_run "$shell" "$profile" "$W/call.inc" "$OUT/wrapper-$shell.out" || die "the claude wrapper failed in $shell: $(cat "$OUT/wrapper-$shell.out")"
    assert_same_file "$OUT/wrapper-$shell.out" "$W/expected-args.txt" "$shell ($profile): the arguments reach claude intact"
done

SBPATH="$FAKEBIN:$TH/bin:$BASEPATH" sb env FAKEBIN="$FAKEBIN" sh -c '. "$1"; . "$2"' wrapper "$HOMEDIR/.profile" "$W/call-notokred.inc" >"$OUT/wrapper-notokred.out" 2>&1 </dev/null ||
    die "the wrapper failed with tokred off PATH: $(cat "$OUT/wrapper-notokred.out")"
assert_same_file "$OUT/wrapper-notokred.out" "$W/expected-args.txt" "with tokred removed from PATH, claude still runs with its arguments"

# ------------------------------------------------------------------ 3. traffic through the proxy

step "3. traffic through the proxy, the dashboard and the metrics"
"$PY" -I "$FAKE" --port-file "$W/fake.port" >"$W/fake.log" 2>&1 &
FAKE_PID=$!
i=0
while [ ! -s "$W/fake.port" ] && [ "$i" -lt 50 ]; do
    i=$((i + 1))
    sleep 0.2
done
[ -s "$W/fake.port" ] || die "the fake upstream did not start: $(cat "$W/fake.log")"
FAKE_PORT=$(cat "$W/fake.port")
assert_eq "$(http_code "http://127.0.0.1:$FAKE_PORT/__fake/health")" "200" "the fake upstream answers on port $FAKE_PORT"

# [proxy] upstream -> the fake, then restart the proxy so it reads it.
sed "s#^upstream = .*#upstream = \"http://127.0.0.1:$FAKE_PORT\"#" "$DATA/tokred.toml" >"$W/toml.new"
cat "$W/toml.new" >"$DATA/tokred.toml"
chk toml "$DATA/tokred.toml" proxy upstream "\"http://127.0.0.1:$FAKE_PORT\""
tk proxy-stop proxy stop
rc_is 0 "tokred proxy stop succeeded" proxy-stop
if wait_down; then ok "the proxy stopped"; else die "the proxy still answers after proxy stop"; fi
tk enable-1 proxy enable --port "$PORT" --record-content true
rc_is 0 "tokred proxy enable --port $PORT --record-content true succeeded" enable-1
if wait_health; then ok "the proxy answers again on port $PORT"; else die "the proxy did not come back on port $PORT"; fi
chk toml "$DATA/tokred.toml" proxy log_bodies true
chk toml "$DATA/tokred.toml" proxy bind "\"127.0.0.1:$PORT\""

SID=smoke-s1
MAIN_MODEL=claude-sonnet-4-5
SUB_MODEL=claude-haiku-4-5
AGENT=smoke-agent-1
SYSTEM_PROMPT="You are a smoke test"
MAIN_PROMPT="hello main loop"
SUB_PROMPT="find where main is"
MAIN_TEXT="fake-reply: hello main loop ok"
SUB_TEXT="fake-reply: find where main is"
cat >"$W/req-main.json" <<EOF
{"model":"$MAIN_MODEL","max_tokens":64,"stream":true,"system":"$SYSTEM_PROMPT","tools":[{"name":"Read","description":"read","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":"$MAIN_PROMPT"}]}
EOF
cat >"$W/req-sub.json" <<EOF
{"model":"$SUB_MODEL","max_tokens":64,"tools":[{"name":"Grep","description":"grep","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":"$SUB_PROMPT"}]}
EOF

# send_msg BODYFILE OUTFILE SESSION [curl args]: a Claude Code style request through the proxy.
send_msg() {
    body=$1
    outfile=$2
    session=$3
    shift 3
    curl -s -N --max-time 30 -D "$outfile.hdr" -o "$outfile" -w '%{http_code} %{time_starttransfer} %{time_total}\n' \
        "http://127.0.0.1:$PORT/v1/messages" -X POST \
        -H 'content-type: application/json' -H 'anthropic-version: 2023-06-01' -H 'x-api-key: smoke-test-not-a-key' \
        -H "x-claude-code-session-id: $session" "$@" --data-binary "@$body" >"$outfile.timing" || die "the request to the proxy failed ($body)"
}

send_msg "$W/req-main.json" "$OUT/reply-main.sse" "$SID"
assert_eq "$(awk '{ print $1 }' "$OUT/reply-main.sse.timing")" "200" "the main-loop request answered 200 through the proxy"
chk sse "$OUT/reply-main.sse" --text "$MAIN_TEXT" --model "$MAIN_MODEL"
if grep -qi '^x-fake-upstream: 1' "$OUT/reply-main.sse.hdr"; then ok "the reply carries the fake upstream's header (the proxy forwarded it)"; else die "the reply lacks x-fake-upstream: the proxy answered by itself?"; fi
if awk '{ exit !(($3 - $2) >= 0.25) }' "$OUT/reply-main.sse.timing"; then ok "the reply was streamed: first byte at $(awk '{ print $2 }' "$OUT/reply-main.sse.timing") s, last at $(awk '{ print $3 }' "$OUT/reply-main.sse.timing") s"; else die "the reply was buffered, not streamed: timing $(cat "$OUT/reply-main.sse.timing")"; fi

send_msg "$W/req-sub.json" "$OUT/reply-sub.json" "$SID" -H "x-claude-code-agent-id: $AGENT"
assert_eq "$(awk '{ print $1 }' "$OUT/reply-sub.json.timing")" "200" "the subagent request answered 200 through the proxy"
chk reply "$OUT/reply-sub.json" --text "$SUB_TEXT" --model "$SUB_MODEL"

# The dashboard: a one-time URL, a cookie, the API.
dash_url() {
    sb "$TOKRED" dashboard --no-open </dev/null 2>"$OUT/dashboard.err" | grep -o 'http://[^ ]*token=[0-9a-f]*' | head -n 1
}
dash_login() {
    url=$(dash_url)
    [ -n "$url" ] || die "tokred dashboard --no-open printed no address: $(cat "$OUT/dashboard.err" 2>/dev/null)"
    rm -f "$JAR"
    code=$(curl -s -o /dev/null --max-time 10 -D "$OUT/redeem.hdr" -c "$JAR" -w '%{http_code}' "$url" || true)
    DASH_URL=$url
    DASH_CODE=$code
}
dash_get() { curl -s --max-time 10 -b "$JAR" -o "$2" -w '%{http_code}' "http://127.0.0.1:$PORT$1"; }

dash_login
assert_eq "$DASH_CODE" "303" "redeeming the one-time dashboard URL answers 303"
assert_has "$OUT/redeem.hdr" "location: /dashboard" "the redirect goes to /dashboard"
assert_has "$JAR" "tokred_dash" "the redeem set the session cookie"
assert_eq "$(http_code "$DASH_URL")" "401" "the one-time token cannot be used twice"
assert_eq "$(http_code "http://127.0.0.1:$PORT/dashboard/api/sessions")" "401" "without a cookie the dashboard API is 401"
assert_eq "$(dash_get /dashboard/api/sessions "$OUT/sessions.json")" "200" "with the cookie the sessions API answers 200"
chk sessions "$OUT/sessions.json" --session "$SID" --requests 2 --models "$MAIN_MODEL,$SUB_MODEL"
assert_eq "$(dash_get "/dashboard/api/sessions/$SID" "$OUT/session-s1.json")" "200" "the session detail answers 200"
chk turns "$OUT/session-s1.json" --session "$SID" --content true --agent-id "$AGENT" \
    --main-model "$MAIN_MODEL" --sub-model "$SUB_MODEL" --system "$SYSTEM_PROMPT" \
    --main-sent "$MAIN_PROMPT" --main-text "$MAIN_TEXT" --sub-sent "$SUB_PROMPT" --sub-text "$SUB_TEXT"
assert_eq "$(dash_get /metrics "$OUT/metrics.html")" "200" "/metrics answers 200 with the cookie"
if grep -qi '<html' "$OUT/metrics.html"; then ok "/metrics is an HTML page"; else die "/metrics is not HTML: $(head -c 200 "$OUT/metrics.html")"; fi
assert_eq "$(http_code -b "$JAR" -H 'Host: evil.example' "http://127.0.0.1:$PORT/dashboard/api/sessions")" "421" "a request with a wrong Host is 421"

tk stats stats --json
rc_is 0 "tokred stats --json succeeded" stats
chk stats "$OUT/stats.out" --requests 2 --p95-under 10
tk stats-text stats
rc_is 0 "tokred stats succeeded" stats-text
assert_has "$OUT/stats-text.out" "Proxy" "tokred stats prints the Proxy section"
assert_has "$OUT/stats-text.out" "2 handled" "the Proxy section says 2 requests were handled"
tk routing routing status
rc_is 0 "tokred routing status runs" routing
assert_has "$OUT/routing.out" "Mode:" "tokred routing status prints the mode"
tk pstatus proxy status --json
rc_is 0 "tokred proxy status --json succeeded" pstatus
chk proxy-status "$OUT/pstatus.out" --running true --port "$PORT"

# A conversation that grows: only the new message is `sent`.
cat >"$W/req-s2a.json" <<EOF
{"model":"$MAIN_MODEL","max_tokens":64,"system":"$SYSTEM_PROMPT","tools":[{"name":"Read","description":"read","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":"first question"}]}
EOF
cat >"$W/req-s2b.json" <<EOF
{"model":"$MAIN_MODEL","max_tokens":64,"system":"$SYSTEM_PROMPT","tools":[{"name":"Read","description":"read","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":"first question"},{"role":"assistant","content":"first answer"},{"role":"user","content":"second question"}]}
EOF
send_msg "$W/req-s2a.json" "$OUT/reply-s2a.json" smoke-s2
send_msg "$W/req-s2b.json" "$OUT/reply-s2b.json" smoke-s2
assert_eq "$(dash_get /dashboard/api/sessions/smoke-s2 "$OUT/session-s2.json")" "200" "the second session's detail answers 200"
chk sent "$OUT/session-s2.json" --session smoke-s2 --expect "assistant=first answer" --expect "user=second question" --count 2

# Content recording off: nothing is recorded (checked on a NEW session, right after the switch).
tk enable-2 proxy enable --record-content false
rc_is 0 "tokred proxy enable --record-content false succeeded" enable-2
chk toml "$DATA/tokred.toml" proxy log_bodies false
cat >"$W/req-s3.json" <<EOF
{"model":"$MAIN_MODEL","max_tokens":64,"tools":[{"name":"Read","description":"read","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":"private words"}]}
EOF
send_msg "$W/req-s3.json" "$OUT/reply-s3.json" smoke-s3
assert_eq "$(dash_get /dashboard/api/sessions/smoke-s3 "$OUT/session-s3.json")" "200" "the new session's detail answers 200"
chk null-content "$OUT/session-s3.json" --session smoke-s3

# And after a restart (so the later steps start from a known state whatever the above found).
tk proxy-stop proxy stop
rc_is 0 "tokred proxy stop succeeded" proxy-stop
if wait_down; then ok "the proxy stopped"; else die "the proxy still answers after proxy stop"; fi
tk enable-3 proxy enable --record-content false
rc_is 0 "tokred proxy enable --record-content false (after a restart) succeeded" enable-3
if wait_health; then ok "the proxy is back"; else die "the proxy did not come back"; fi
cat >"$W/req-s4.json" <<EOF
{"model":"$MAIN_MODEL","max_tokens":64,"tools":[{"name":"Read","description":"read","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":"private words again"}]}
EOF
send_msg "$W/req-s4.json" "$OUT/reply-s4.json" smoke-s4
dash_login
assert_eq "$DASH_CODE" "303" "a new dashboard login after the restart answers 303"
assert_eq "$(dash_get /dashboard/api/sessions/smoke-s4 "$OUT/session-s4.json")" "200" "the session detail answers 200 after the restart"
chk null-content "$OUT/session-s4.json" --session smoke-s4

# ------------------------------------------------------------------ 4. unconfigure / configure

step "4. unconfigure and configure"
printf '\n# edited by the user\n' >>"$CLAUDE/agents/tokred-dev.md"
cp "$CLAUDE/agents/tokred-dev.md" "$W/dev-edited.md"
tk unconfigure unconfigure
rc_is 0 "tokred unconfigure succeeded" unconfigure
chk settings "$CLAUDE/settings.json" --hooks absent --user --url "http://127.0.0.1:$PORT"
for a in architect planner reviewer explorer summarizer; do
    assert_missing "$CLAUDE/agents/tokred-$a.md" "unconfigure removed the agent tokred-$a.md"
done
assert_same_file "$CLAUDE/agents/tokred-dev.md" "$W/dev-edited.md" "the agent the user edited (tokred-dev.md) is kept untouched"
assert_eq "$(health_code)" "200" "unconfigure left the proxy running"
chk marker "$DATA/tokred.db" --empty

# A "newer binary" simulation: the marker is empty, so other commands must not reinstall anything.
tk doctor doctor
rc_is 0 "tokred doctor runs" doctor
tk init initialize
rc_is 0 "tokred initialize runs" init
tk stats2 stats
rc_is 0 "tokred stats runs" stats2
chk settings "$CLAUDE/settings.json" --hooks absent --user --url "http://127.0.0.1:$PORT"
assert_missing "$CLAUDE/agents/tokred-architect.md" "doctor/initialize/stats did not put an agent back"
chk marker "$DATA/tokred.db" --empty

tk configure configure --yes
rc_is 0 "tokred configure --yes succeeded" configure
chk settings "$CLAUDE/settings.json" --hooks present --bin "$TOKRED" --user --url "http://127.0.0.1:$PORT"
chk agents "$CLAUDE/agents" --count 6
assert_same_file "$CLAUDE/agents/tokred-dev.md" "$W/dev-edited.md" "configure kept the edited agent"
assert_file "$CLAUDE/agents/tokred-dev.md.new" "configure wrote the new version of the edited agent next to it (.new)"
chk marker "$DATA/tokred.db" --is "$VERSION"

# ------------------------------------------------------------------ 5. the proxy switch

step "5. proxy disable / enable"
tk disable proxy disable
rc_is 0 "tokred proxy disable succeeded" disable
chk settings "$CLAUDE/settings.json" --hooks present --user --no-url
chk toml "$DATA/tokred.toml" proxy enabled false
assert_eq "$(health_code)" "200" "disable leaves a running proxy running"

wrapper_run sh .profile "$W/call.inc" "$OUT/wrapper-disabled.out" || die "the wrapper failed while disabled: $(cat "$OUT/wrapper-disabled.out")"
assert_same_file "$OUT/wrapper-disabled.out" "$W/expected-args.txt" "claude through the wrapper still runs while the proxy is disabled"
chk settings "$CLAUDE/settings.json" --no-url
chk toml "$DATA/tokred.toml" proxy enabled false
ok "opening claude through the wrapper did not re-enable the proxy"

tk enable-4 proxy enable
rc_is 0 "tokred proxy enable (no flags) succeeded" enable-4
chk toml "$DATA/tokred.toml" proxy bind "\"127.0.0.1:$PORT\""
chk toml "$DATA/tokred.toml" proxy enabled true
chk settings "$CLAUDE/settings.json" --user --url "http://127.0.0.1:$PORT"
assert_eq "$(health_code)" "200" "the proxy is still answering on the saved port $PORT"

FOREIGN="https://gateway.example.test/anthropic"
chk edit "$CLAUDE/settings.json" --set-url "$FOREIGN"
tk init-foreign initialize
rc_is 0 "tokred initialize with a foreign ANTHROPIC_BASE_URL exits 0" init-foreign
assert_has "$OUT/init-foreign.out" "leaves it alone" "initialize warns that the foreign ANTHROPIC_BASE_URL is left alone"
chk settings "$CLAUDE/settings.json" --foreign "$FOREIGN"
tk disable-foreign proxy disable
rc_is 0 "tokred proxy disable with a foreign URL succeeded" disable-foreign
chk settings "$CLAUDE/settings.json" --foreign "$FOREIGN" --user --hooks present
chk edit "$CLAUDE/settings.json" --drop-url
tk enable-5 proxy enable
rc_is 0 "tokred proxy enable brings the URL back" enable-5
chk settings "$CLAUDE/settings.json" --url "http://127.0.0.1:$PORT" --user

# ------------------------------------------------------------------ 6. update

step "6. tokred update against the GitHub release API"
if [ "${SMOKE_SKIP_UPDATE:-0}" = "1" ]; then
    skip "step 6 skipped (SMOKE_SKIP_UPDATE=1)"
else
    if [ -n "${GITHUB_TOKEN:-}" ]; then
        probe=$(curl -s -o /dev/null --max-time 20 -H "Authorization: Bearer $GITHUB_TOKEN" -w '%{http_code}' "$RELEASES_API" || true)
    else
        probe=$(curl -s -o /dev/null --max-time 20 -w '%{http_code}' "$RELEASES_API" || true)
    fi
    EXTRA_ENV="TOKRED_RELEASES_URL=$RELEASES_API"
    tk update update
    EXTRA_ENV="TOKRED_OFFLINE=1"
    case "$probe" in
        404)
            note "$REPO has only pre-releases (GitHub's latest answers 404)"
            assert_eq "$RC" "1" "tokred update exits 1 when there is no stable release"
            assert_has "$OUT/update.out" "no stable release" "tokred update says that no stable release exists"
            ;;
        200)
            note "$REPO has a stable release: the update branch is unverified by design (it may replace the binary)"
            if grep -q "up to date" "$OUT/update.out"; then
                assert_eq "$RC" "0" "tokred update says it is up to date and exits 0"
            elif grep -q "Updating\|Updated to\|is available" "$OUT/update.out"; then
                assert_eq "$RC" "0" "tokred update offered or applied a newer version and exits 0"
            elif grep -qi "rate limit\|403\|429" "$OUT/update.out"; then
                skip "tokred update was rate limited by the GitHub API"
            else
                die "tokred update said something unexpected: $(head -c 300 "$OUT/update.out")"
            fi
            ;;
        *) skip "the GitHub API answered '$probe' to the probe (rate limit or no network): step 6 not checked" ;;
    esac
    # The binary must still run after an update attempt.
    if sb "$TOKRED" --version >"$OUT/version-after-update.out" 2>&1 </dev/null; then ok "the binary still runs after tokred update ($(cat "$OUT/version-after-update.out"))"; else die "the binary does not run after tokred update"; fi
fi

# ------------------------------------------------------------------ 7. uninstall, both ways

# assert_clean LABEL [edited-agent]: nothing of tokred is left. With the second argument the one agent
# the user edited (tokred-dev.md) is expected to be kept, untouched, as `unconfigure` promises.
assert_clean() {
    label=$1
    edited=${2:-}
    i=0
    while [ -e "$TH" ] && [ "$i" -lt 50 ]; do
        i=$((i + 1))
        sleep 0.3
    done
    assert_missing "$TH" "$label: \$TOKRED_HOME is gone"
    assert_missing "$DATA" "$label: the data folder is gone"
    chk settings "$CLAUDE/settings.json" --hooks absent --no-url --user --equals "$ORIG/settings.json"
    if [ -n "$edited" ]; then
        chk agents "$CLAUDE/agents" --only dev
        assert_same_file "$CLAUDE/agents/tokred-dev.md" "$W/dev-edited.md" "$label: the agent the user edited was kept untouched"
    else
        chk agents "$CLAUDE/agents" --none
    fi
    for f in .profile .bashrc .zshrc; do
        assert_eq "$(blocks_in "$HOMEDIR/$f")" "0" "$label: $f has no tokred block"
        assert_same_file "$HOMEDIR/$f" "$ORIG/$f" "$label: $f is byte-for-byte what the user had"
    done
    if wait_down; then ok "$label: nothing answers on port $PORT"; else die "$label: something still answers on port $PORT"; fi
    sleep 1
    strays=$(ps_lines | grep -F "$W/" | grep -v "grep" | grep -v "smoke.sh" | grep -v "fake_upstream.py" || true)
    if [ -z "$strays" ]; then ok "$label: no tokred process is left"; else die "$label: processes are left: $strays"; fi
    (cd "$HOMEDIR" && find . | sort) >"$W/home.now"
    assert_same_file "$W/home.now" "$ORIG/home.list" "$label: no stray file in the sandbox HOME"
    # The Claude folder: a backup of settings.json and the (empty) agents folder are tokred's own
    # leftovers by design; anything else is a stray.
    (cd "$CLAUDE" && find . -mindepth 1 | sort) >"$W/claude.now"
    extra=$(grep -v -x -e './settings.json' -e './settings.json.tokred.bak' -e './agents' -e "${edited:+./agents/tokred-dev.md}" "$W/claude.now" || true)
    if [ -z "$extra" ]; then ok "$label: the Claude folder has only settings.json and tokred's settings backup"; else die "$label: stray files in the Claude folder: $extra"; fi
    if grep -qx './settings.json.tokred.bak' "$W/claude.now"; then note "$label: settings.json.tokred.bak (tokred's backup of the user's settings) stays in the Claude folder"; fi
    if [ -d "$CLAUDE/agents" ]; then note "$label: an empty agents/ folder stays in the Claude folder"; fi
}

step "7a. uninstall with the installed binary: tokred uninstall"
tk uninstall1 uninstall
rc_is 0 "tokred uninstall succeeded" uninstall1
assert_clean "tokred uninstall" edited-agent
rm -f "$CLAUDE/agents/tokred-dev.md"

step "7b. reinstall, then uninstall with the release's uninstall.sh"
# Back to the starting state of the user's files (assert_clean proved it is equal), then install again.
RC=0
sb sh "$DL/install.sh" --yes --version "$VERSION" --port "$PORT" >"$OUT/install2.out" 2>&1 </dev/null || RC=$?
rc_is 0 "the second install.sh succeeded" install2
assert_eq "$(sb "$TOKRED" --version </dev/null)" "tokred $VERSION" "the reinstalled tokred --version equals the tag"
chk settings "$CLAUDE/settings.json" --hooks present --bin "$TOKRED" --user --url "http://127.0.0.1:$PORT"
assert_eq "$(health_code)" "200" "the reinstalled proxy answers on port $PORT"
RC=0
sb sh "$DL/uninstall.sh" >"$OUT/uninstall2.out" 2>&1 </dev/null || RC=$?
rc_is 0 "the release's uninstall.sh succeeded" uninstall2
assert_clean "uninstall.sh"

# ------------------------------------------------------------------ done

printf '\n'
if [ "$FAILS" -gt 0 ]; then
    printf 'FAIL: %s assertion(s) failed, %s passed, %s skipped (release %s of %s)\n' "$FAILS" "$ASSERTS" "$SKIPS" "$TAG" "$REPO" >&2
    exit 1
fi
printf 'PASS: %s assertions passed, %s skipped (release %s of %s)\n' "$ASSERTS" "$SKIPS" "$TAG" "$REPO"
exit 0
