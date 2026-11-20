#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd -P)"
readonly REPO_ROOT

fail() {
    printf '%s\n' "$1" >&2
    exit 1
}

command -v git >/dev/null 2>&1 || fail 'required command not found: git'

test_root="$(mktemp -d "${TMPDIR:-/tmp}/soma-identity-test.XXXXXX")"
test_root="$(cd "$test_root" && pwd -P)"
cleanup() {
    find "$test_root" -depth -delete
}
trap cleanup EXIT

checkout="$test_root/checkout"
mkdir -p "$checkout/scripts"
cp "$REPO_ROOT/scripts/build-identity.sh" "$checkout/scripts/"
identity() {
    /bin/bash "$checkout/scripts/build-identity.sh" "$@"
}

value() {
    local name="$1"
    identity --print | sed -n "s/^${name}=//p"
}

# Outside any checkout the identity is absent, never invented, even when the caller's own
# environment carried an old one.
[[ -z "$(SOMA_BUILD_COMMIT=stale identity --print)" ]] ||
    fail 'a directory outside Git must yield no identity'
inherited="$(SOMA_BUILD_SOURCE=clean identity /usr/bin/env | grep '^SOMA_BUILD_' || true)"
[[ -z "$inherited" ]] || fail "an inherited identity survived: ${inherited}"

(
    cd "$checkout"
    git init -q
    git config user.email 'soma-identity-test@example.invalid'
    git config user.name 'SOMA identity test'
    printf 'ignored\n' > .gitignore
    printf 'one\n' > tracked.txt
    git add .
    GIT_COMMITTER_DATE='2026-11-20T18:04:11+05:00' git commit -qm 'identity fixture'
)
commit="$(git -C "$checkout" rev-parse HEAD)"

[[ "$(value SOMA_BUILD_COMMIT)" == "$commit" ]] || fail 'the commit is not the checkout HEAD'
[[ "$(value SOMA_BUILD_COMMIT_DATE)" == '2026-11-20T13:04:11Z' ]] ||
    fail "the commit date is not the committer date in UTC: $(value SOMA_BUILD_COMMIT_DATE)"
[[ "$(value SOMA_BUILD_SOURCE)" == clean ]] || fail 'a clean checkout must be clean'

printf 'build output\n' > "$checkout/ignored"
[[ "$(value SOMA_BUILD_SOURCE)" == clean ]] || fail 'an ignored file must not make the tree dirty'

printf 'two\n' > "$checkout/tracked.txt"
[[ "$(value SOMA_BUILD_SOURCE)" == dirty ]] || fail 'a modified tracked file must be dirty'
git -C "$checkout" checkout -q -- tracked.txt

printf 'new\n' > "$checkout/untracked.txt"
[[ "$(value SOMA_BUILD_SOURCE)" == dirty ]] || fail 'an untracked file must be dirty'

# A command run through the script sees the same three values the print mode reports.
seen="$(identity /usr/bin/env | grep '^SOMA_BUILD_' | LC_ALL=C sort)"
expected="$(identity --print | LC_ALL=C sort)"
[[ "$seen" == "$expected" ]] || fail "the command saw a different identity: ${seen}"

# A checkout nested in another repository does not borrow the outer commit.
nested="$checkout/nested"
mkdir -p "$nested/scripts"
cp "$REPO_ROOT/scripts/build-identity.sh" "$nested/scripts/"
[[ -z "$(/bin/bash "$nested/scripts/build-identity.sh" --print)" ]] ||
    fail 'a nested directory must not report the enclosing repository'

printf 'build identity tests passed\n'
