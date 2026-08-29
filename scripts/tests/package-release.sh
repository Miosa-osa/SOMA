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

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        fail "required command not found: $1"
    fi
}

require_command cargo
require_command git
require_command jq

test_root="$(mktemp -d "${TMPDIR:-/tmp}/soma-package-test.XXXXXX")"
cleanup() {
    find "$test_root" -depth -delete
}
trap cleanup EXIT

fixture="$test_root/repository"
output="$test_root/output"
mkdir -p \
    "$fixture/crates/public-base/src" \
    "$fixture/crates/public-fixture/src" \
    "$fixture/crates/private-fixture/src" \
    "$fixture/scripts"

cp "$REPO_ROOT/LICENSE" "$REPO_ROOT/NOTICE" "$fixture/"
cp "$REPO_ROOT/LICENSE" "$REPO_ROOT/NOTICE" "$fixture/crates/public-base/"
cp "$REPO_ROOT/LICENSE" "$REPO_ROOT/NOTICE" "$fixture/crates/public-fixture/"
cp "$REPO_ROOT/scripts/package-release.sh" "$fixture/scripts/"
cp "$REPO_ROOT/scripts/verify-release-artifacts.sh" "$fixture/scripts/"

printf '%s\n' '1.0.0' > "$fixture/VERSION"
printf '%s\n' \
    '[workspace]' \
    'members = ["crates/public-base", "crates/public-fixture", "crates/private-fixture"]' \
    'resolver = "3"' \
    > "$fixture/Cargo.toml"
printf '%s\n' \
    '[package]' \
    'name = "public-base"' \
    'description = "SOMA release-packager public dependency fixture"' \
    'version = "1.0.0"' \
    'edition = "2024"' \
    'license = "Apache-2.0"' \
    'repository = "https://example.invalid/soma-release-test"' \
    > "$fixture/crates/public-base/Cargo.toml"
printf '%s\n' 'pub fn dependency() {}' > "$fixture/crates/public-base/src/lib.rs"
printf '%s\n' \
    '[package]' \
    'name = "public-fixture"' \
    'description = "SOMA release-packager public test fixture"' \
    'version = "1.0.0"' \
    'edition = "2024"' \
    'license = "Apache-2.0"' \
    'repository = "https://example.invalid/soma-release-test"' \
    '' \
    '[dependencies]' \
    'public-base = { path = "../public-base", version = "=1.0.0" }' \
    > "$fixture/crates/public-fixture/Cargo.toml"
printf '%s\n' 'pub use public_base::dependency as packaged;' \
    > "$fixture/crates/public-fixture/src/lib.rs"
printf '%s\n' \
    '[package]' \
    'name = "private-fixture"' \
    'version = "1.0.0"' \
    'edition = "2024"' \
    'publish = false' \
    > "$fixture/crates/private-fixture/Cargo.toml"
printf '%s\n' 'pub fn private() {}' > "$fixture/crates/private-fixture/src/lib.rs"
printf '%s\n' 'compile_error!("private crate must not be packaged");' \
    > "$fixture/crates/private-fixture/build.rs"

(
    cd "$fixture"
    cargo generate-lockfile
    git init -q
    git config user.email 'soma-release-test@example.invalid'
    git config user.name 'SOMA release test'
    git add .
    git commit -qm 'test fixture'
    SOMA_RELEASE_TAG=v1.0.0 \
        SOMA_RELEASE_OUTPUT="$output" \
        /bin/bash scripts/package-release.sh
)

[[ -f "$output/public-fixture-1.0.0.crate" ]] ||
    fail 'public crate archive is missing'
[[ -f "$output/public-base-1.0.0.crate" ]] ||
    fail 'public dependency crate archive is missing'
[[ ! -e "$output/private-fixture-1.0.0.crate" ]] ||
    fail 'private crate archive must not be released'
[[ -f "$output/BUILD-INFO.txt" ]] || fail 'build information is missing'
[[ -f "$output/SHA256SUMS" ]] || fail 'release checksums are missing'
if find "$output" -mindepth 1 -maxdepth 1 -type f -name '*private*' | grep -q .; then
    fail 'private package leaked into the release output'
fi

printf 'release packager regression test passed\n'
