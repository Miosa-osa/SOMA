#!/usr/bin/env bash
#
# Verifies SOMA release artifacts the way a deployer must before installing anything.
#
#   verify-release-artifacts.sh source  DIR TAG
#   verify-release-artifacts.sh bundle  DIR TAG TARGET KIND BINARY...   (KIND: client or host)
#   verify-release-artifacts.sh client  DIR TAG TARGET CLI_BINARY MCP_BINARY
#   verify-release-artifacts.sh release DIR TAG
#
# `release` checks a published release directory: SHA256SUMS over exactly its files, the
# manifest, every archive it names, and every binary's digest and size inside its archive.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
# shellcheck source=scripts/release/common.sh
source "${REPO_ROOT}/scripts/release/common.sh"
# shellcheck source=scripts/release/verify-release.sh
source "${REPO_ROOT}/scripts/release/verify-release.sh"
cd "$REPO_ROOT"

# Every extraction lands here, and nothing outlives the run.
VERIFY_WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/soma-verify.XXXXXX")"
readonly VERIFY_WORK_DIR
cleanup() {
    rm -rf -- "$VERIFY_WORK_DIR"
}
trap cleanup EXIT

# Checks that ARCHIVE lists exactly the expected paths, all regular files under one directory.
verify_archive_listing() {
    local archive="$1"
    local root="$2"
    shift 2
    local expected actual

    expected="$( { printf '%s/\n' "$root"; for name in "$@"; do printf '%s/%s\n' "$root" "$name"; done; } |
        LC_ALL=C sort)"
    actual="$(tar -tzf "$archive" | LC_ALL=C sort)"
    if [[ "$expected" != "$actual" ]]; then
        diff -u <(printf '%s\n' "$expected") <(printf '%s\n' "$actual") >&2 || true
        fail "archive has an unexpected structure: ${archive}"
    fi
    [[ "$(tar -tvzf "$archive" "${root}/" | awk 'NR == 1 { print $1 }')" == d* ]] ||
        fail "archive root must be a directory: ${archive}"
}

verify_source() {
    local directory="$1"
    local release_tag="$2"
    local archive_name="soma-${release_tag}-source.tar.gz"
    local root="soma-${release_tag}"
    local temp_dir

    [[ -d "$directory" ]] || fail "source release directory is missing: ${directory}"
    require_command tar
    verify_checksum_manifest "$directory"
    [[ "$(find "$directory" -mindepth 1 -maxdepth 1 -type f ! -name SHA256SUMS -exec basename {} \;)" == \
        "$archive_name" ]] || fail 'the source part must hold only the source archive and SHA256SUMS'
    if tar -tzf "$directory/$archive_name" | grep -v -q "^${root}/"; then
        fail "source archive has a path outside ${root}/"
    fi
    temp_dir="$VERIFY_WORK_DIR/source"
    mkdir -p "$temp_dir"
    tar -xzf "$directory/$archive_name" -C "$temp_dir" "${root}/VERSION" "${root}/LICENSE" "${root}/NOTICE"
    [[ "v$(tr -d '\r\n' < "$temp_dir/$root/VERSION")" == "$release_tag" ]] ||
        fail 'the source archive is not the tagged version'
    for legal_file in LICENSE NOTICE; do
        cmp -s "$legal_file" "$temp_dir/$root/$legal_file" ||
            fail "source archive ${legal_file} differs from the repository"
    done
    printf 'source release artifact verified: %s\n' "$directory/$archive_name"
}

verify_bundle() {
    local directory="$1"
    local release_tag="$2"
    local release_target="$3"
    local kind="$4"
    shift 4
    local bundle_name archive records temp_dir root name mode record

    case "$kind" in
        client) bundle_name="soma-${release_tag}-${release_target}" ;;
        host) bundle_name="soma-host-${release_tag}-${release_target}" ;;
        *) fail 'bundle KIND must be client or host' ;;
    esac
    (( $# > 0 )) || fail 'a bundle names at least one binary'
    archive="$directory/${bundle_name}.tar.gz"
    records="$directory/${bundle_name}.binaries.json"
    [[ -d "$directory" ]] || fail "bundle directory is missing: ${directory}"
    require_command jq
    require_command tar
    verify_checksum_manifest "$directory"
    [[ "$(find "$directory" -mindepth 1 -maxdepth 1 -type f -exec basename {} \; | LC_ALL=C sort)" == \
        "$(printf '%s\n' SHA256SUMS "${bundle_name}.binaries.json" "${bundle_name}.tar.gz" | LC_ALL=C sort)" ]] ||
        fail 'a bundle directory holds only its archive, its binary records, and SHA256SUMS'
    verify_archive_listing "$archive" "$bundle_name" BUILD-INFO.txt LICENSE NOTICE SHA256SUMS "$@"
    for name in BUILD-INFO.txt LICENSE NOTICE SHA256SUMS "$@"; do
        mode="$(tar -tvzf "$archive" "${bundle_name}/${name}" | awk 'NR == 1 { print $1 }')"
        [[ "$mode" == -* ]] || fail "archive entry must be a regular file: ${name}"
    done
    for name in "$@"; do
        mode="$(tar -tvzf "$archive" "${bundle_name}/${name}" | awk 'NR == 1 { print $1 }')"
        [[ "$mode" == -rwxr-xr-x* ]] ||
            fail "archive does not preserve executable mode for ${name}: ${mode:-missing}"
    done
    temp_dir="$VERIFY_WORK_DIR/bundle-${bundle_name}"
    mkdir -p "$temp_dir"
    tar -xzf "$archive" -C "$temp_dir"
    root="$temp_dir/$bundle_name"
    cmp -s LICENSE "$root/LICENSE" || fail 'bundle LICENSE differs from the repository'
    cmp -s NOTICE "$root/NOTICE" || fail 'bundle NOTICE differs from the repository'
    grep -Fxq "release_tag=${release_tag}" "$root/BUILD-INFO.txt" ||
        fail 'BUILD-INFO.txt does not identify the release tag'
    grep -Fxq "target=${release_target}" "$root/BUILD-INFO.txt" ||
        fail 'BUILD-INFO.txt does not identify the target'
    verify_checksum_manifest "$root"
    [[ "$(jq -r '[.[].name] | sort | join(" ")' "$records")" == \
        "$(printf '%s\n' "$@" | LC_ALL=C sort | paste -sd ' ' -)" ]] ||
        fail 'the binary records do not name exactly the bundled binaries'
    for name in "$@"; do
        record="$(jq -c --arg name "$name" '.[] | select(.name == $name)' "$records")"
        verify_binary_record "$record" "$root/$name" "${bundle_name}.tar.gz" \
            "${bundle_name}/${name}" "${release_tag#v}"
    done
    printf '%s bundle verified: %s\n' "$kind" "$archive"
}

usage() {
    sed -n '3,9p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
}

case "${1:-}" in
    source)
        (( $# == 3 )) || { usage; exit 2; }
        verify_source "$2" "$3"
        ;;
    bundle)
        (( $# >= 6 )) || { usage; exit 2; }
        shift
        verify_bundle "$@"
        ;;
    client)
        (( $# == 6 )) || { usage; exit 2; }
        verify_bundle "$2" "$3" "$4" client "$5" "$6"
        ;;
    release)
        (( $# == 3 )) || { usage; exit 2; }
        verify_release "$2" "$3"
        ;;
    *)
        usage
        exit 2
        ;;
esac
