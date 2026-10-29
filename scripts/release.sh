#!/bin/sh
# Cuts a release candidate and promotes it to the final version.
#
#   scripts/release.sh rc      0.2.0-rc.1     bump to the RC, move the changelog, commit, tag, push
#   scripts/release.sh promote 0.2.0-rc.1     turn a tested RC into 0.2.0 (same commit + the version)
#
# Both end with a tag push, which starts the `Release` workflow (the binaries) and, when it
# succeeds, `Release smoke` (the real install from the published assets). `promote` refuses
# unless that smoke test passed for the RC and nothing changed on main since the RC.
#
# Options: -n/--dry-run (checks and edits the files, but does not commit, tag or push),
# -h/--help. Needs: git, python3, and `gh` logged in (reads the CI and smoke results).
# Environment (tests and CI): RELEASE_SKIP_LOCK=1 does not refresh Cargo.lock;
# RELEASE_SKIP_CI_CHECK=1 does not ask GitHub (the tests of this script use it).

set -eu

# Files are edited before the checks that can still fail: put them back when one does.
EDITING=0
die() {
    printf 'error: %s\n' "$*" >&2
    if [ "$EDITING" = "1" ]; then git checkout -- . 2>/dev/null || true; fi
    exit 1
}
say() { printf '%s\n' "$*"; }
step() { printf '==> %s\n' "$*"; }

usage() {
    sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
}

DRY=0
ACTION=
VERSION=
while [ $# -gt 0 ]; do
    case "$1" in
        -n | --dry-run) DRY=1 ;;
        -h | --help) usage; exit 0 ;;
        rc | promote)
            [ -z "$ACTION" ] || die "give one action only"
            ACTION=$1
            ;;
        -*) die "unknown option: $1 (try --help)" ;;
        *)
            [ -z "$VERSION" ] || die "give one version only"
            VERSION=${1#v}
            ;;
    esac
    shift
done
[ -n "$ACTION" ] && [ -n "$VERSION" ] || { usage >&2; exit 1; }

# 0.2.0-rc.3 -> final 0.2.0, N 3
case "$VERSION" in
    [0-9]*.[0-9]*.[0-9]*-rc.[0-9]*) ;;
    *) die "'$VERSION' is not a release candidate version like 0.2.0-rc.1" ;;
esac
FINAL=${VERSION%-rc.*}
TODAY=$(date -u +%Y-%m-%d)
ROOT=$(git rev-parse --show-toplevel) || die "not inside a git repository"
cd "$ROOT"
[ -f Cargo.toml ] || die "no Cargo.toml at $ROOT"

# ---------------------------------------------------------------- helpers

current_version() {
    sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1
}

# set_version FROM TO: the workspace version and the version of every path dependency.
set_version() {
    python3 - "$1" "$2" <<'PY'
import re, sys
old, new = sys.argv[1], sys.argv[2]
text = open("Cargo.toml").read()
count = text.count(f'version = "{old}"')
if count == 0:
    sys.exit(f"Cargo.toml has no version = \"{old}\"")
open("Cargo.toml", "w").write(text.replace(f'version = "{old}"', f'version = "{new}"'))
PY
    if [ "${RELEASE_SKIP_LOCK:-0}" != "1" ]; then
        cargo update --workspace --quiet || die "could not refresh Cargo.lock"
    fi
}

# changelog_rc FILE UNRELEASED_HEADING VERSION DATE: moves the Unreleased entries under a new
# section; refuses when there is nothing to release.
changelog_rc() {
    python3 - "$1" "$2" "$3" "$4" <<'PY'
import re, sys
path, heading, version, date = sys.argv[1:5]
text = open(path).read()
start = text.find(f"## [{heading}]")
if start < 0:
    sys.exit(f"{path}: no '## [{heading}]' section")
after = start + len(f"## [{heading}]")
nxt = text.find("\n## [", after)
nxt = len(text) if nxt < 0 else nxt + 1
body = text[after:nxt]
if not re.search(r"^- ", body, re.M):
    sys.exit(f"{path}: the '{heading}' section has no entries: nothing to release")
new = f"## [{heading}]\n\n## [{version}] - {date}{body}"
open(path, "w").write(text[:start] + new + text[nxt:])
PY
}

# changelog_final FILE RC FINAL DATE: the RC section becomes the final one.
changelog_final() {
    python3 - "$1" "$2" "$3" "$4" <<'PY'
import re, sys
path, rc, final, date = sys.argv[1:5]
text = open(path).read()
pattern = re.compile(r"^## \[" + re.escape(rc) + r"\] - .*$", re.M)
if not pattern.search(text):
    sys.exit(f"{path}: no section for {rc}")
open(path, "w").write(pattern.sub(f"## [{final}] - {date}", text, count=1))
PY
}

clean_tree() {
    [ -z "$(git status --porcelain)" ] || die "the working tree has uncommitted changes"
}

on_main_in_sync() {
    [ "$(git rev-parse --abbrev-ref HEAD)" = "main" ] || die "run this on the main branch"
    git fetch origin --tags --quiet || die "could not fetch from origin"
    [ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] \
        || die "main is not the same as origin/main: pull or push first"
}

tag_exists() { git rev-parse -q --verify "refs/tags/$1" >/dev/null; }

finish() { # finish VERSION
    if [ "$DRY" = "1" ]; then
        say ""
        say "Dry run: the files are edited and nothing was committed, tagged or pushed."
        say "(Undo with: git checkout -- .)"
        return 0
    fi
    EDITING=0
    git add -A
    git commit -q -m "chore: release $1" || die "could not commit"
    git tag "v$1" || die "could not create the tag v$1"
    git push origin main || die "could not push main"
    git push origin "v$1" || die "could not push the tag v$1"
    say ""
    say "Pushed v$1. The Release workflow builds it; Release smoke tests it afterwards."
}

# ---------------------------------------------------------------- rc

do_rc() {
    clean_tree
    on_main_in_sync
    tag_exists "v$VERSION" && die "the tag v$VERSION already exists"
    OLD=$(current_version)
    [ -n "$OLD" ] || die "could not read the version from Cargo.toml"
    EDITING=1
    step "Bumping $OLD -> $VERSION"
    set_version "$OLD" "$VERSION" || die "could not set the version"
    step "Moving the changelog entries under $VERSION"
    changelog_rc CHANGELOG.md Unreleased "$VERSION" "$TODAY" || die "CHANGELOG.md was not updated"
    changelog_rc CHANGELOG.pt-BR.md "Não lançado" "$VERSION" "$TODAY" || die "CHANGELOG.pt-BR.md was not updated"
    finish "$VERSION"
}

# ---------------------------------------------------------------- promote

gh_json() { gh "$@" || die "gh failed: gh $*"; }

check_github() {
    [ "${RELEASE_SKIP_CI_CHECK:-0}" = "1" ] && return 0
    command -v gh >/dev/null 2>&1 || die "the GitHub CLI (gh) is needed to read the CI and smoke results"
    sha=$(git rev-parse "v$VERSION^{commit}")

    step "The RC exists as a pre-release"
    pre=$(gh_json release view "v$VERSION" --json isPrerelease --jq .isPrerelease)
    [ "$pre" = "true" ] || die "v$VERSION is not published as a pre-release on GitHub"

    step "CI passed on the RC commit"
    ok=$(gh_json run list --workflow CI --commit "$sha" --json conclusion,event \
        --jq '[.[]|select(.event=="push" and .conclusion=="success")]|length')
    [ "$ok" -ge 1 ] || die "no successful CI run on $sha"

    step "Release smoke passed for v$VERSION"
    state=$(gh_json run list --workflow release-smoke.yml --limit 50 --json displayTitle,conclusion \
        --jq "[.[]|select(.displayTitle==\"Release smoke v$VERSION\")]|.[0].conclusion // \"none\"")
    [ "$state" = "success" ] \
        || die "Release smoke for v$VERSION is '$state': run it (Actions > Release smoke > Run workflow) and wait for it to pass"
}

do_promote() {
    clean_tree
    on_main_in_sync
    tag_exists "v$VERSION" || die "the tag v$VERSION does not exist"
    tag_exists "v$FINAL" && die "the tag v$FINAL already exists"
    [ "$(git rev-parse HEAD)" = "$(git rev-parse "v$VERSION^{commit}")" ] \
        || die "main moved since v$VERSION: cut a new RC with what changed, test it, then promote that one"
    [ "$(current_version)" = "$VERSION" ] || die "Cargo.toml is at $(current_version), not $VERSION"
    check_github
    EDITING=1
    step "Setting the version $VERSION -> $FINAL"
    set_version "$VERSION" "$FINAL" || die "could not set the version"
    changelog_final CHANGELOG.md "$VERSION" "$FINAL" "$TODAY" || die "CHANGELOG.md was not updated"
    changelog_final CHANGELOG.pt-BR.md "$VERSION" "$FINAL" "$TODAY" || die "CHANGELOG.pt-BR.md was not updated"
    finish "$FINAL"
}

case "$ACTION" in
    rc) do_rc ;;
    promote) do_promote ;;
esac
