#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly TAG_PATTERN='^v[0-9A-Za-z.+-]+$'

cd "${REPO_ROOT}"

fail() {
    printf '%s\n' "$1" >&2
    exit 1
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        fail "required command not found: $1"
    fi
}

release_tag="${SOMA_RELEASE_TAG:-}"
allow_dirty="${SOMA_RELEASE_ALLOW_DIRTY:-0}"

if [[ ! "$release_tag" =~ $TAG_PATTERN ]]; then
    fail 'SOMA_RELEASE_TAG must be a SemVer tag such as v1.0.0 or v1.0.0-alpha.1'
fi

if [[ "$allow_dirty" != "0" && "$allow_dirty" != "1" ]]; then
    fail 'SOMA_RELEASE_ALLOW_DIRTY must be 0 or 1'
fi

release_version="${release_tag#v}"
output_dir="${SOMA_RELEASE_OUTPUT:-${REPO_ROOT}/target/release-bundles/${release_tag}}"

require_command cargo
require_command git
require_command jq
require_command rustc

if [[ ! -f VERSION ]]; then
    fail 'VERSION is missing'
fi
source_version="$(tr -d '\r\n' < VERSION)"
if [[ -z "$source_version" || "$source_version" != "$release_version" ]]; then
    fail "VERSION is ${source_version:-empty}, expected ${release_version}"
fi
if [[ "$(wc -l < VERSION | tr -d ' ')" != "1" ]]; then
    fail 'VERSION must contain exactly one version line'
fi

metadata="$(cargo metadata --locked --format-version 1 --no-deps)"
package_count=0
while IFS=$'\t' read -r package_name package_version; do
    package_count=$((package_count + 1))
    if [[ "$package_version" != "$release_version" ]]; then
        fail "workspace package ${package_name} is ${package_version}, expected ${release_version}"
    fi
done < <(
    printf '%s\n' "$metadata" |
        jq -r '.workspace_members as $members | .packages[] | select(.id as $id | $members | index($id)) | [.name, .version] | @tsv'
)

if (( package_count == 0 )); then
    fail 'cargo metadata reported no workspace packages'
fi

package_arguments=(--workspace --locked)
if [[ "$allow_dirty" == "1" ]]; then
    package_arguments+=(--allow-dirty)
else
    if ! git rev-parse --verify HEAD >/dev/null 2>&1; then
        fail 'release packaging requires a committed revision'
    fi
    if [[ -n "$(git status --porcelain=v1 --untracked-files=normal)" ]]; then
        fail 'release packaging requires a clean working tree'
    fi
fi

if [[ -e "$output_dir" ]]; then
    fail "release output already exists: ${output_dir}"
fi

stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/soma-release.XXXXXX")"
cleanup() {
    rm -rf -- "$stage_dir"
}
trap cleanup EXIT

mkdir -p "$output_dir"
public_package_count=0
while IFS= read -r package_name; do
    public_package_count=$((public_package_count + 1))
done < <(
    printf '%s\n' "$metadata" |
        jq -r '.workspace_members as $members | .packages[] | select(.id as $id | $members | index($id)) | select(.publish != []) | .name'
)

if (( public_package_count == 0 )); then
    fail 'cargo metadata reported no public workspace packages'
fi

while IFS= read -r package_name; do
    package_arguments+=(--exclude "$package_name")
done < <(
    printf '%s\n' "$metadata" |
        jq -r '.workspace_members as $members | .packages[] | select(.id as $id | $members | index($id)) | select(.publish == []) | .name'
)

cargo package --target-dir "$stage_dir" "${package_arguments[@]}"

while IFS=$'\t' read -r package_name package_version; do
    archive="${stage_dir}/package/${package_name}-${package_version}.crate"
    if [[ ! -f "$archive" ]]; then
        fail "cargo did not produce expected archive: ${archive}"
    fi
    cp "$archive" "$output_dir/"
done < <(
    printf '%s\n' "$metadata" |
        jq -r '.workspace_members as $members | .packages[] | select(.id as $id | $members | index($id)) | select(.publish != []) | [.name, .version] | @tsv'
)

if git rev-parse --verify HEAD >/dev/null 2>&1; then
    revision="$(git rev-parse HEAD)"
else
    revision="uncommitted"
fi

{
    printf 'release_tag=%s\n' "$release_tag"
    printf 'revision=%s\n' "$revision"
    rustc --version
    cargo --version
} > "$output_dir/BUILD-INFO.txt"

checksum_file="$output_dir/SHA256SUMS"
: > "$checksum_file"
while IFS= read -r artifact; do
    artifact_name="${artifact##*/}"
    if command -v sha256sum >/dev/null 2>&1; then
        (cd "$output_dir" && sha256sum "$artifact_name") >> "$checksum_file"
    elif command -v shasum >/dev/null 2>&1; then
        (cd "$output_dir" && shasum -a 256 "$artifact_name") >> "$checksum_file"
    else
        fail 'sha256sum or shasum is required to generate release checksums'
    fi
done < <(find "$output_dir" -maxdepth 1 -type f ! -name SHA256SUMS -print | LC_ALL=C sort)

"${REPO_ROOT}/scripts/verify-release-artifacts.sh" crates "$output_dir"

printf 'release bundle ready: %s\n' "$output_dir"
