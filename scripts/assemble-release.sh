#!/usr/bin/env bash
#
# Assembles release parts into the one directory a release publishes, and writes its manifest.
#
#   SOMA_RELEASE_TAG=v1.0.0 scripts/assemble-release.sh OUTPUT_DIR PART_DIR...
#
# A part is a directory whose SHA256SUMS covers exactly its files: the source part, or one
# bundle with its `.binaries.json` records. Every part is verified before anything is copied.
# The output holds every archive, the release manifest `soma-TAG-manifest.json`, and one
# SHA256SUMS over all of them, and it is verified as a deployer would before this returns.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly MANIFEST_SCHEMA="soma.release-manifest.v1"
# shellcheck source=scripts/release/common.sh
source "${REPO_ROOT}/scripts/release/common.sh"
cd "$REPO_ROOT"

(( $# >= 2 )) || fail 'usage: assemble-release.sh OUTPUT_DIR PART_DIR...'
output_dir="$1"
shift
release_tag="${SOMA_RELEASE_TAG:-}"
[[ "$release_tag" =~ ^v[0-9A-Za-z.+-]+$ ]] || fail 'SOMA_RELEASE_TAG must be a tag such as v1.0.0'
require_command jq
"$REPO_ROOT/scripts/check-version.sh" >/dev/null
release_version="$(tr -d '\r\n' < VERSION)"
[[ "$release_tag" == "v${release_version}" ]] ||
    fail "release tag ${release_tag} does not match VERSION ${release_version}"
[[ ! -e "$output_dir" ]] || fail "release output already exists: ${output_dir}"

identity="$("$REPO_ROOT/scripts/build-identity.sh" --print)"
commit="$(printf '%s\n' "$identity" | sed -n 's/^SOMA_BUILD_COMMIT=//p')"
commit_date="$(printf '%s\n' "$identity" | sed -n 's/^SOMA_BUILD_COMMIT_DATE=//p')"
[[ -n "$commit" && -n "$commit_date" ]] || fail 'assembly needs the Git checkout of the release'
rust="$(sed -n -E 's/^channel = "([^"]+)"$/\1/p' rust-toolchain.toml)"

stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/soma-assemble.XXXXXX")"
cleanup() {
    rm -rf -- "$stage_dir"
}
trap cleanup EXIT
records="$stage_dir/records.json"
printf '[]\n' > "$records"
mkdir -p "$stage_dir/release"
for part in "$@"; do
    [[ -d "$part" ]] || fail "release part is not a directory: ${part}"
    verify_checksum_manifest "$part"
    while IFS= read -r name; do
        case "$name" in
            *.binaries.json)
                jq -s 'add' "$records" "$part/$name" > "$stage_dir/merged.json"
                mv "$stage_dir/merged.json" "$records"
                ;;
            *)
                [[ ! -e "$stage_dir/release/$name" ]] || fail "two parts ship ${name}"
                cp "$part/$name" "$stage_dir/release/$name"
                ;;
        esac
    done < <(checksum_manifest_names "$part/SHA256SUMS")
done

manifest_name="soma-${release_tag}-manifest.json"
artifacts="$(
    find "$stage_dir/release" -mindepth 1 -maxdepth 1 -type f -exec basename {} \; |
        LC_ALL=C sort |
        while IFS= read -r name; do
            jq -n --arg file "$name" --arg sha256 "$(sha256_of "$stage_dir/release/$name")" \
                --argjson size "$(size_of "$stage_dir/release/$name")" \
                '{file: $file, sha256: $sha256, size: $size}'
        done | jq -s .
)"
jq -n \
    --arg schema "$MANIFEST_SCHEMA" \
    --arg release "$release_version" \
    --arg tag "$release_tag" \
    --arg commit "$commit" \
    --arg commit_date "$commit_date" \
    --arg rust "$rust" \
    --argjson artifacts "$artifacts" \
    --slurpfile records "$records" \
    '($records[0] | sort_by(.target, .name)) as $binaries
     | {schema: $schema, release: $release, tag: $tag,
        prerelease: ($release | contains("-")),
        commit: $commit, commit_date: $commit_date, rust: $rust,
        api_version: ([$binaries[] | select(.name == "soma-api") | .api_version] | first),
        artifacts: $artifacts, binaries: $binaries}' \
    > "$stage_dir/release/$manifest_name"

files=()
while IFS= read -r name; do
    files+=("$name")
done < <(find "$stage_dir/release" -mindepth 1 -maxdepth 1 -type f -exec basename {} \; |
    LC_ALL=C sort)
write_checksum_manifest "$stage_dir/release" SHA256SUMS "${files[@]}"
"$REPO_ROOT/scripts/verify-release-artifacts.sh" release "$stage_dir/release" "$release_tag"
mkdir -p "$(dirname "$output_dir")"
mv "$stage_dir/release" "$output_dir"
printf 'release assembled: %s\n' "$output_dir"
