#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd -P)"
readonly REPO_ROOT
# shellcheck source=scripts/release/common.sh
source "${REPO_ROOT}/scripts/release/common.sh"
require_command cargo

test_root="$(mktemp -d "${TMPDIR:-/tmp}/soma-set-version-test.XXXXXX")"
cleanup() {
    find "$test_root" -depth -delete
}
trap cleanup EXIT

fixture="$test_root/repository"
mkdir -p "$fixture/scripts/release" "$fixture/crates/base/src" "$fixture/crates/user/src"
cp "$REPO_ROOT/scripts/set-version.sh" "$REPO_ROOT/scripts/check-version.sh" "$fixture/scripts/"
cp "$REPO_ROOT/scripts/release/common.sh" "$fixture/scripts/release/"
printf '1.0.0-alpha.1\n' > "$fixture/VERSION"
printf '%s\n' \
    '[workspace]' \
    'members = ["crates/base", "crates/user"]' \
    'resolver = "3"' \
    '' \
    '[workspace.package]' \
    'rust-version = "1.98"' \
    'version = "1.0.0-alpha.1"' \
    > "$fixture/Cargo.toml"
printf '%s\n' '[package]' 'name = "base"' 'version.workspace = true' 'edition = "2024"' \
    > "$fixture/crates/base/Cargo.toml"
printf '%s\n' '[package]' 'name = "user"' 'version.workspace = true' 'edition = "2024"' '' \
    '[dependencies]' 'base = { path = "../base", version = "=1.0.0-alpha.1" }' \
    > "$fixture/crates/user/Cargo.toml"
printf '\n' > "$fixture/crates/base/src/lib.rs"
printf '\n' > "$fixture/crates/user/src/lib.rs"
(cd "$fixture" && cargo generate-lockfile --offline --quiet)

(cd "$fixture" && ./scripts/set-version.sh 1.0.0-alpha.2 > "$test_root/out")

[[ "$(cat "$fixture/VERSION")" == 1.0.0-alpha.2 ]] || fail 'VERSION did not move'
grep -qx 'version = "1.0.0-alpha.2"' "$fixture/Cargo.toml" || fail 'the workspace version did not move'
grep -qx 'rust-version = "1.98"' "$fixture/Cargo.toml" || fail 'an unrelated version line changed'
grep -qF 'version = "=1.0.0-alpha.2"' "$fixture/crates/user/Cargo.toml" ||
    fail 'the workspace requirement did not move'
(cd "$fixture" && cargo metadata --locked --offline --format-version 1 --no-deps > /dev/null) ||
    fail 'Cargo.lock does not match the moved manifests'
grep -q '1 crate manifests' "$test_root/out" || fail "unexpected summary: $(cat "$test_root/out")"

for refused in 1.0 v1.0.0 1.0.0+build 1.0.0-alpha.2; do
    if (cd "$fixture" && ./scripts/set-version.sh "$refused" > /dev/null 2>&1); then
        fail "set-version accepted ${refused}"
    fi
done

printf 'set-version tests passed\n'
