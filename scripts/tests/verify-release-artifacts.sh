#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd -P)"
readonly REPO_ROOT
readonly VERIFIER="${REPO_ROOT}/scripts/verify-release-artifacts.sh"

cd "$REPO_ROOT"

fail() {
    printf '%s\n' "$1" >&2
    exit 1
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        fail "required command not found: $1"
    fi
}

write_binary_manifest() {
    local directory="$1"
    local manifest_name="$2"
    shift 2
    local file

    : > "$directory/$manifest_name"
    for file in "$@"; do
        (cd "$directory" && sha256sum --binary -- "$file") >> "$directory/$manifest_name"
    done
}

require_command cmp
require_command grep
require_command sha256sum
require_command tar

test_root="$(mktemp -d "${TMPDIR:-/tmp}/soma-verifier-test.XXXXXX")"
cleanup() {
    find "$test_root" -depth -delete
}
trap cleanup EXIT

release_tag="v0.0.0-test"
release_target="x86_64-pc-windows-msvc"
cli_binary="soma.exe"
mcp_binary="soma-mcp.exe"
bundle_name="soma-${release_tag}-${release_target}"
archive_name="${bundle_name}.tar.gz"
stage_dir="$test_root/stage"
bundle_dir="$stage_dir/$bundle_name"
delivery_dir="$test_root/delivery"
bad_delivery_dir="$test_root/bad-delivery"
mkdir -p "$bundle_dir" "$delivery_dir" "$bad_delivery_dir"

cp LICENSE NOTICE "$bundle_dir/"
{
    printf 'release_tag=%s\n' "$release_tag"
    printf 'target=%s\n' "$release_target"
    printf 'revision=test-fixture\n'
} > "$bundle_dir/BUILD-INFO.txt"
printf 'test CLI binary\n' > "$bundle_dir/$cli_binary"
printf 'test MCP binary\n' > "$bundle_dir/$mcp_binary"
chmod 0755 "$bundle_dir/$cli_binary" "$bundle_dir/$mcp_binary"

write_binary_manifest "$bundle_dir" SHA256SUMS \
    BUILD-INFO.txt LICENSE NOTICE "$cli_binary" "$mcp_binary"
tar -czf "$delivery_dir/$archive_name" -C "$stage_dir" "$bundle_name"
write_binary_manifest "$delivery_dir" SHA256SUMS "$archive_name"

grep -Eq '^[0-9a-f]{64} \*' "$bundle_dir/SHA256SUMS" ||
    fail 'inner fixture manifest does not contain binary-mode checksum markers'
grep -Eq '^[0-9a-f]{64} \*' "$delivery_dir/SHA256SUMS" ||
    fail 'outer fixture manifest does not contain binary-mode checksum markers'
cp "$bundle_dir/SHA256SUMS" "$test_root/inner-before"
cp "$delivery_dir/SHA256SUMS" "$test_root/outer-before"

"$VERIFIER" client \
    "$delivery_dir" "$release_tag" "$release_target" "$cli_binary" "$mcp_binary"

cmp -s "$test_root/inner-before" "$bundle_dir/SHA256SUMS" ||
    fail 'inner checksum manifest changed during verification'
cmp -s "$test_root/outer-before" "$delivery_dir/SHA256SUMS" ||
    fail 'outer checksum manifest changed during verification'

cp "$delivery_dir/$archive_name" "$bad_delivery_dir/"
archive_digest="$(sha256sum -- "$bad_delivery_dir/$archive_name" | awk '{ print $1 }')"
printf '%s **%s\n' "$archive_digest" "$archive_name" > "$bad_delivery_dir/SHA256SUMS"
if "$VERIFIER" client \
    "$bad_delivery_dir" "$release_tag" "$release_target" "$cli_binary" "$mcp_binary" \
    > "$test_root/bad-verifier.log" 2>&1; then
    fail 'verifier removed more than one leading binary-mode marker'
fi
grep -Fq 'checksum manifest does not cover exactly the shipped files' \
    "$test_root/bad-verifier.log" ||
    fail 'verifier rejected the double marker for an unexpected reason'

printf 'release artifact verifier regression tests passed\n'
