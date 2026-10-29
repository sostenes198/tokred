#!/bin/sh
# Tests for scripts/release.sh (cutting a release candidate and promoting it).
#
#   sh tests/release_scripts/test.sh
#
# Each case runs in a throwaway git repository with a bare repository as `origin`, and a fake `gh`
# that answers from FAKE_* variables, so nothing here touches GitHub or the real repository.

set -eu

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SCRIPT="$ROOT/scripts/release.sh"
WORK_ROOT=$(mktemp -d 2>/dev/null || mktemp -d -t tokred-release-test)
trap 'rm -rf "$WORK_ROOT"' EXIT

ok() { printf 'ok - %s\n' "$*"; }
die() { printf 'not ok - %s\n' "$*" >&2; exit 1; }
assert_has() { grep -qF -- "$2" "$1" || die "$1 should contain: $2"; }
assert_lacks() { ! grep -qF -- "$2" "$1" || die "$1 should not contain: $2"; }
assert_eq() { [ "$1" = "$2" ] || die "$3: expected '$2', got '$1'"; }

# A fake gh: `release view` answers FAKE_PRE, `run list --workflow CI` FAKE_CI, and the smoke
# workflow FAKE_SMOKE.
FAKE_BIN="$WORK_ROOT/bin"
mkdir -p "$FAKE_BIN"
cat >"$FAKE_BIN/gh" <<'EOF'
#!/bin/sh
case "$*" in
    "release view"*) printf '%s\n' "${FAKE_PRE:-true}" ;;
    *"--workflow CI"*) printf '%s\n' "${FAKE_CI:-1}" ;;
    *"--workflow release-smoke.yml"*) printf '%s\n' "${FAKE_SMOKE:-success}" ;;
    *) exit 1 ;;
esac
EOF
chmod +x "$FAKE_BIN/gh"

# new_repo NAME: a repository at version 0.1.0 with changelog entries, pushed to a bare origin.
new_repo() {
    dir="$WORK_ROOT/$1"
    mkdir -p "$dir"
    git init -q --bare "$dir/origin.git"
    git init -q -b main "$dir/work"
    cd "$dir/work"
    git config user.email test@example.com
    git config user.name test
    cat >Cargo.toml <<'EOF'
[workspace.package]
version = "0.1.0"

[workspace.dependencies]
tokred-core = { path = "crates/tokred-core", version = "0.1.0" }
tokred-proxy = { path = "crates/tokred-proxy", version = "0.1.0" }
EOF
    cat >CHANGELOG.md <<'EOF'
# Changelog

## [Unreleased]

### Added

- A new thing.

## [0.0.9] - 2026-01-01

- Old.
EOF
    cat >CHANGELOG.pt-BR.md <<'EOF'
# Changelog

## [Não lançado]

### Adicionado

- Uma coisa nova.

## [0.0.9] - 2026-01-01

- Antigo.
EOF
    git add -A
    git commit -q -m "initial"
    git remote add origin "$dir/origin.git"
    git push -q origin main
    git fetch -q origin
    echo "$dir/work"
}

run() { PATH="$FAKE_BIN:$PATH" RELEASE_SKIP_LOCK=1 sh "$SCRIPT" "$@"; }
fails_with() { # fails_with MESSAGE cmd...
    msg=$1
    shift
    if out=$("$@" 2>&1); then die "should have failed: $*"; fi
    printf '%s' "$out" | grep -qF -- "$msg" || die "expected '$msg' in: $out"
}

# ------------------------------------------------------------ 1. an RC
repo=$(new_repo one)
cd "$repo"
run rc 0.2.0-rc.1 >/dev/null || die "the rc failed"
assert_eq "$(grep -c 'version = "0.2.0-rc.1"' Cargo.toml)" 3 "the workspace and both dependencies are bumped"
assert_has CHANGELOG.md "## [Unreleased]"
assert_has CHANGELOG.md "## [0.2.0-rc.1] - "
assert_eq "$(sed -n '/^## \[Unreleased\]/,/^## \[0.2.0-rc.1\]/p' CHANGELOG.md | grep -c '^- ')" 0 "Unreleased is empty now"
assert_has CHANGELOG.pt-BR.md "## [0.2.0-rc.1] - "
assert_has CHANGELOG.pt-BR.md "- Uma coisa nova."
assert_eq "$(git log -1 --format=%s)" "chore: release 0.2.0-rc.1" "the commit"
assert_eq "$(git rev-parse HEAD)" "$(git rev-parse v0.2.0-rc.1^{commit})" "the tag is on the commit"
assert_eq "$(git rev-parse origin/main)" "$(git rev-parse HEAD)" "main was pushed"
git ls-remote --tags origin | grep -q "refs/tags/v0.2.0-rc.1" || die "the tag was not pushed"
ok "an RC bumps the version, moves the changelog, commits, tags and pushes"

fails_with "already exists" run rc 0.2.0-rc.1
ok "the same RC is not cut twice"

fails_with "nothing to release" run rc 0.2.0-rc.2
assert_eq "$(git status --porcelain)" "" "a refused RC leaves the files as they were"
assert_eq "$(grep -c 'version = "0.2.0-rc.1"' Cargo.toml)" 3 "and the version unchanged"
ok "an RC with no changelog entries is refused and nothing is left edited"

fails_with "not a release candidate" run rc 0.2.0
fails_with "not a release candidate" run promote 0.2.0
ok "only 0.x.y-rc.n versions are accepted"

# ------------------------------------------------------------ 2. promote
echo more >>CHANGELOG.md
git commit -qam "a change after the RC"
git push -q origin main
fails_with "main moved" run promote 0.2.0-rc.1
ok "promote refuses when main moved since the RC"

repo=$(new_repo two)
cd "$repo"
run rc 0.2.0-rc.1 >/dev/null || die "the rc failed"
run promote 0.2.0-rc.1 >/dev/null || die "the promotion failed"
assert_eq "$(grep -c 'version = "0.2.0"' Cargo.toml)" 3 "the final version"
assert_has CHANGELOG.md "## [0.2.0] - "
assert_lacks CHANGELOG.md "0.2.0-rc.1"
assert_has CHANGELOG.pt-BR.md "## [0.2.0] - "
assert_eq "$(git log -1 --format=%s)" "chore: release 0.2.0" "the commit"
assert_eq "$(git rev-parse origin/main)" "$(git rev-parse HEAD)" "main was pushed"
git ls-remote --tags origin | grep -q "refs/tags/v0.2.0$" || die "the final tag was not pushed"
ok "promote turns the RC into the final version, commits, tags and pushes"

fails_with "already exists" run promote 0.2.0-rc.1
ok "a version is promoted once"

repo=$(new_repo three)
cd "$repo"
fails_with "does not exist" run promote 0.2.0-rc.1
ok "promote needs the RC tag"

# ------------------------------------------------------------ 3. the GitHub checks
repo=$(new_repo four)
cd "$repo"
run rc 0.2.0-rc.1 >/dev/null
export RELEASE_SKIP_CI_CHECK=
fails_with "not published as a pre-release" env FAKE_PRE=false PATH="$FAKE_BIN:$PATH" RELEASE_SKIP_LOCK=1 sh "$SCRIPT" promote 0.2.0-rc.1
fails_with "no successful CI run" env FAKE_CI=0 PATH="$FAKE_BIN:$PATH" RELEASE_SKIP_LOCK=1 sh "$SCRIPT" promote 0.2.0-rc.1
fails_with "Release smoke for v0.2.0-rc.1 is 'failure'" env FAKE_SMOKE=failure PATH="$FAKE_BIN:$PATH" RELEASE_SKIP_LOCK=1 sh "$SCRIPT" promote 0.2.0-rc.1
fails_with "is 'none'" env FAKE_SMOKE=none PATH="$FAKE_BIN:$PATH" RELEASE_SKIP_LOCK=1 sh "$SCRIPT" promote 0.2.0-rc.1
assert_eq "$(git status --porcelain)" "" "a refused promotion leaves the files as they were"
ok "promote refuses a missing pre-release, a red CI and a smoke test that did not pass"
PATH="$FAKE_BIN:$PATH" RELEASE_SKIP_LOCK=1 sh "$SCRIPT" promote 0.2.0-rc.1 >/dev/null || die "a green RC was not promoted"
ok "promote goes ahead when everything is green"

# ------------------------------------------------------------ 4. other guards
repo=$(new_repo five)
cd "$repo"
echo x >dirty
fails_with "uncommitted changes" run rc 0.2.0-rc.1
rm dirty
git checkout -q -b other
fails_with "main branch" run rc 0.2.0-rc.1
git checkout -q main
ok "a dirty tree and another branch are refused"

repo=$(new_repo six)
cd "$repo"
run --dry-run rc 0.2.0-rc.1 >/dev/null || die "the dry run failed"
assert_eq "$(git log -1 --format=%s)" "initial" "a dry run does not commit"
if git rev-parse -q --verify refs/tags/v0.2.0-rc.1 >/dev/null; then die "a dry run tagged"; fi
assert_has Cargo.toml '0.2.0-rc.1'
ok "a dry run edits the files and does nothing else"

printf 'all release script tests passed\n'
