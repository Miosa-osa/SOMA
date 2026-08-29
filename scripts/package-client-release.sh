#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT

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

write_checksum_manifest() {
    local directory="$1"
    local manifest_name="$2"
    shift 2
    local file

    : > "$directory/$manifest_name"
    for file in "$@"; do
        if command -v sha256sum >/dev/null 2>&1; then
            (cd "$directory" && sha256sum "$file") >> "$directory/$manifest_name"
        elif command -v shasum >/dev/null 2>&1; then
            (cd "$directory" && shasum -a 256 "$file") >> "$directory/$manifest_name"
        else
            fail 'sha256sum or shasum is required to generate release checksums'
        fi
    done
}

release_tag="${SOMA_RELEASE_TAG:-}"
release_target="${SOMA_RELEASE_TARGET:-}"
cli_binary="${SOMA_RELEASE_CLI_BINARY:-}"
mcp_binary="${SOMA_RELEASE_MCP_BINARY:-}"
allow_dirty="${SOMA_RELEASE_ALLOW_DIRTY:-0}"

if [[ ! "$release_tag" =~ ^v[0-9A-Za-z.+-]+$ ]]; then
    fail 'SOMA_RELEASE_TAG must be a version tag such as v1.0.0 or v1.0.0-alpha.1'
fi
if [[ ! "$release_target" =~ ^[0-9A-Za-z_.-]+$ ]]; then
    fail 'SOMA_RELEASE_TARGET must be a safe Rust target triple'
fi
for binary in "$cli_binary" "$mcp_binary"; do
    if [[ ! "$binary" =~ ^[0-9A-Za-z_.-]+$ ]]; then
        fail 'release binary names must be safe base names'
    fi
done
if [[ "$cli_binary" == "$mcp_binary" ]]; then
    fail 'CLI and MCP release binary names must be distinct'
fi
if [[ "$allow_dirty" != "0" && "$allow_dirty" != "1" ]]; then
    fail 'SOMA_RELEASE_ALLOW_DIRTY must be 0 or 1'
fi

require_command cargo
require_command git
require_command rustc
require_command tar
"$REPO_ROOT/scripts/check-version.sh" >/dev/null
release_version="$(tr -d '\r\n' < VERSION)"
if [[ "$release_tag" != "v${release_version}" ]]; then
    fail "release tag ${release_tag} does not match VERSION ${release_version}"
fi

if [[ "$allow_dirty" == "0" ]]; then
    git rev-parse --verify HEAD >/dev/null 2>&1 || fail 'client packaging requires a committed revision'
    if [[ -n "$(git status --porcelain=v1 --untracked-files=normal)" ]]; then
        fail 'client packaging requires a clean working tree'
    fi
fi

bundle_name="soma-${release_tag}-${release_target}"
output_dir="${SOMA_RELEASE_OUTPUT:-${REPO_ROOT}/target/release-clients/${release_tag}/${release_target}}"
if [[ -e "$output_dir" ]]; then
    fail "client release output already exists: ${output_dir}"
fi

cargo build --release --locked --target "$release_target" -p soma-cli -p soma-mcp
binary_dir="${REPO_ROOT}/target/${release_target}/release"
for binary in "$cli_binary" "$mcp_binary"; do
    if [[ ! -f "$binary_dir/$binary" ]]; then
        fail "built client binary is missing: ${binary_dir}/${binary}"
    fi
done

stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/soma-client-release.XXXXXX")"
cleanup() {
    rm -rf -- "$stage_dir"
}
trap cleanup EXIT

bundle_dir="$stage_dir/$bundle_name"
delivery_dir="$stage_dir/delivery"
mkdir -p "$bundle_dir" "$delivery_dir"
cp "$binary_dir/$cli_binary" "$bundle_dir/$cli_binary"
cp "$binary_dir/$mcp_binary" "$bundle_dir/$mcp_binary"
chmod 0755 "$bundle_dir/$cli_binary" "$bundle_dir/$mcp_binary"
cp LICENSE NOTICE "$bundle_dir/"

if git rev-parse --verify HEAD >/dev/null 2>&1; then
    revision="$(git rev-parse HEAD)"
else
    revision="uncommitted"
fi
{
    printf 'release_tag=%s\n' "$release_tag"
    printf 'target=%s\n' "$release_target"
    printf 'revision=%s\n' "$revision"
    rustc --version
    cargo --version
} > "$bundle_dir/BUILD-INFO.txt"

write_checksum_manifest "$bundle_dir" SHA256SUMS \
    BUILD-INFO.txt LICENSE NOTICE "$cli_binary" "$mcp_binary"
archive_name="${bundle_name}.tar.gz"
tar -czf "$delivery_dir/$archive_name" -C "$stage_dir" "$bundle_name"
write_checksum_manifest "$delivery_dir" SHA256SUMS "$archive_name"

"$REPO_ROOT/scripts/verify-release-artifacts.sh" client \
    "$delivery_dir" "$release_tag" "$release_target" "$cli_binary" "$mcp_binary"
mkdir -p "$(dirname "$output_dir")"
mv "$delivery_dir" "$output_dir"
printf 'client release bundle ready: %s\n' "$output_dir"
