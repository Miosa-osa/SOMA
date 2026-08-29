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

checksum_manifest_names() {
    local manifest="$1"
    local output="$2"
    local digest filename extra

    : > "$output"
    while read -r digest filename extra; do
        if [[ ! "$digest" =~ ^[0-9a-f]{64}$ || -z "$filename" || -n "${extra:-}" ]]; then
            fail "invalid checksum record in ${manifest}"
        fi
        # GNU checksum manifests prefix binary-mode file names with one `*` marker.
        # Normalize only the coverage copy and leave the manifest untouched for --check.
        if [[ "$filename" == \** ]]; then
            filename="${filename#\*}"
        fi
        if [[ -z "$filename" ]]; then
            fail "invalid checksum record in ${manifest}"
        fi
        if [[ "$filename" == -* || "$filename" == */* || "$filename" == *\\* ]]; then
            fail "unsafe checksum path in ${manifest}: ${filename}"
        fi
        printf '%s\n' "$filename" >> "$output"
    done < "$manifest"

    if [[ ! -s "$output" ]]; then
        fail "checksum manifest is empty: ${manifest}"
    fi
    if [[ -n "$(sort "$output" | uniq -d)" ]]; then
        fail "checksum manifest contains duplicate paths: ${manifest}"
    fi
}

directory_file_names() {
    local directory="$1"
    local excluded_name="$2"
    local output="$3"
    local file name

    : > "$output"
    while IFS= read -r file; do
        name="${file##*/}"
        if [[ "$name" != "$excluded_name" ]]; then
            printf '%s\n' "$name" >> "$output"
        fi
    done < <(find "$directory" -mindepth 1 -maxdepth 1 -type f -print)
}

verify_checksum_manifest() {
    local directory="$1"
    local manifest_name="${2:-SHA256SUMS}"
    local manifest="$directory/$manifest_name"
    local temp_dir listed actual unexpected

    [[ -f "$manifest" ]] || fail "checksum manifest is missing: ${manifest}"
    unexpected="$(find "$directory" -mindepth 1 -maxdepth 1 ! -type f -print -quit)"
    if [[ -n "$unexpected" ]]; then
        fail "release directory contains a non-regular entry: ${unexpected}"
    fi
    temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/soma-checksums.XXXXXX")"
    listed="$temp_dir/listed"
    actual="$temp_dir/actual"
    checksum_manifest_names "$manifest" "$listed"
    directory_file_names "$directory" "$manifest_name" "$actual"
    if ! diff -u <(LC_ALL=C sort "$listed") <(LC_ALL=C sort "$actual"); then
        rm -rf -- "$temp_dir"
        fail "checksum manifest does not cover exactly the shipped files in ${directory}"
    fi

    if command -v sha256sum >/dev/null 2>&1; then
        (cd "$directory" && sha256sum --check "$manifest_name")
    elif command -v shasum >/dev/null 2>&1; then
        (cd "$directory" && shasum -a 256 --check "$manifest_name")
    else
        rm -rf -- "$temp_dir"
        fail 'sha256sum or shasum is required to verify release checksums'
    fi
    rm -rf -- "$temp_dir"
}

verify_crate_archive() {
    local archive="$1"
    local package_name="$2"
    local package_version="$3"
    local archive_root="${package_name}-${package_version}"
    local temp_dir listing mode

    temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/soma-crate.XXXXXX")"
    listing="$temp_dir/listing"
    tar -tzf "$archive" > "$listing"
    if ! awk -v prefix="${archive_root}/" 'index($0, prefix) != 1 { exit 1 }' "$listing"; then
        rm -rf -- "$temp_dir"
        fail "crate archive has a path outside ${archive_root}: ${archive}"
    fi

    for legal_file in LICENSE NOTICE; do
        if [[ "$(grep -Fxc "${archive_root}/${legal_file}" "$listing")" != "1" ]]; then
            rm -rf -- "$temp_dir"
            fail "crate archive must contain exactly one ${legal_file}: ${archive}"
        fi
        mode="$(tar -tvzf "$archive" "${archive_root}/${legal_file}" | awk 'NR == 1 { print $1 }')"
        if [[ "$mode" != -* ]]; then
            rm -rf -- "$temp_dir"
            fail "crate archive ${legal_file} must be a regular file: ${archive}"
        fi
        if ! tar -xOzf "$archive" "${archive_root}/${legal_file}" | cmp -s "$legal_file" -; then
            rm -rf -- "$temp_dir"
            fail "crate archive ${legal_file} differs from repository root: ${archive}"
        fi
    done
    rm -rf -- "$temp_dir"
}

verify_crates() {
    local output_dir="$1"
    local metadata temp_dir expected actual package_name package_version archive

    [[ -d "$output_dir" ]] || fail "crate release directory is missing: ${output_dir}"
    require_command cargo
    require_command jq
    require_command tar
    temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/soma-crates.XXXXXX")"
    expected="$temp_dir/expected"
    actual="$temp_dir/actual"
    : > "$expected"
    metadata="$(cargo metadata --locked --format-version 1 --no-deps)"
    while IFS=$'\t' read -r package_name package_version; do
        archive="${package_name}-${package_version}.crate"
        printf '%s\n' "$archive" >> "$expected"
        [[ -f "$output_dir/$archive" ]] || fail "public crate archive is missing: ${archive}"
        verify_crate_archive "$output_dir/$archive" "$package_name" "$package_version"
    done < <(
        printf '%s\n' "$metadata" |
            jq -r '.workspace_members as $members | .packages[] | select(.id as $id | $members | index($id)) | select(.publish != []) | [.name, .version] | @tsv'
    )
    find "$output_dir" -mindepth 1 -maxdepth 1 -type f -name '*.crate' -exec basename {} \; |
        LC_ALL=C sort > "$actual"
    if ! diff -u <(LC_ALL=C sort "$expected") "$actual"; then
        rm -rf -- "$temp_dir"
        fail 'crate release directory does not contain exactly the public workspace crates'
    fi
    printf '%s\n' BUILD-INFO.txt SHA256SUMS >> "$expected"
    find "$output_dir" -mindepth 1 -maxdepth 1 -type f -exec basename {} \; |
        LC_ALL=C sort > "$actual"
    if ! diff -u <(LC_ALL=C sort "$expected") "$actual"; then
        rm -rf -- "$temp_dir"
        fail 'crate release directory contains an unexpected delivery file'
    fi
    verify_checksum_manifest "$output_dir"
    rm -rf -- "$temp_dir"
    printf 'crate release artifacts verified: %s\n' "$output_dir"
}

verify_client() {
    local output_dir="$1"
    local release_tag="$2"
    local release_target="$3"
    local cli_binary="$4"
    local mcp_binary="$5"
    local bundle_name="soma-${release_tag}-${release_target}"
    local archive="$output_dir/${bundle_name}.tar.gz"
    local temp_dir listing expected extracted mode root_mode delivery_files

    [[ -d "$output_dir" ]] || fail "client release directory is missing: ${output_dir}"
    [[ -f "$archive" ]] || fail "client archive is missing: ${archive}"
    require_command tar
    verify_checksum_manifest "$output_dir"
    temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/soma-client.XXXXXX")"
    listing="$temp_dir/listing"
    expected="$temp_dir/expected"
    extracted="$temp_dir/extracted"
    delivery_files="$temp_dir/delivery-files"
    find "$output_dir" -mindepth 1 -maxdepth 1 -type f -exec basename {} \; |
        LC_ALL=C sort > "$delivery_files"
    if ! diff -u <(printf '%s\n' SHA256SUMS "${bundle_name}.tar.gz" | LC_ALL=C sort) "$delivery_files"; then
        rm -rf -- "$temp_dir"
        fail 'client release directory must contain only the archive and its checksum manifest'
    fi
    tar -tzf "$archive" | LC_ALL=C sort > "$listing"
    {
        printf '%s/\n' "$bundle_name"
        for file in BUILD-INFO.txt LICENSE NOTICE SHA256SUMS "$cli_binary" "$mcp_binary"; do
            printf '%s/%s\n' "$bundle_name" "$file"
        done
    } | LC_ALL=C sort > "$expected"
    if ! diff -u "$expected" "$listing"; then
        rm -rf -- "$temp_dir"
        fail "client archive has an unexpected package structure: ${archive}"
    fi

    root_mode="$(tar -tvzf "$archive" "${bundle_name}/" | awk 'NR == 1 { print $1 }')"
    if [[ "$root_mode" != d* ]]; then
        rm -rf -- "$temp_dir"
        fail "client archive root must be a directory: ${archive}"
    fi
    for file in BUILD-INFO.txt LICENSE NOTICE SHA256SUMS "$cli_binary" "$mcp_binary"; do
        mode="$(tar -tvzf "$archive" "${bundle_name}/${file}" | awk 'NR == 1 { print $1 }')"
        if [[ "$mode" != -* ]]; then
            rm -rf -- "$temp_dir"
            fail "client archive entry must be a regular file: ${file}"
        fi
    done
    for binary in "$cli_binary" "$mcp_binary"; do
        mode="$(tar -tvzf "$archive" "${bundle_name}/${binary}" | awk 'NR == 1 { print $1 }')"
        if [[ "$mode" != -rwxr-xr-x* ]]; then
            rm -rf -- "$temp_dir"
            fail "client archive does not preserve executable mode for ${binary}: ${mode:-missing}"
        fi
    done

    mkdir -p "$extracted"
    tar -xzf "$archive" -C "$extracted"
    cmp -s LICENSE "$extracted/$bundle_name/LICENSE" || fail 'client LICENSE differs from repository root'
    cmp -s NOTICE "$extracted/$bundle_name/NOTICE" || fail 'client NOTICE differs from repository root'
    grep -Fxq "release_tag=${release_tag}" "$extracted/$bundle_name/BUILD-INFO.txt" ||
        fail 'client BUILD-INFO.txt does not identify the release tag'
    grep -Fxq "target=${release_target}" "$extracted/$bundle_name/BUILD-INFO.txt" ||
        fail 'client BUILD-INFO.txt does not identify the target'
    verify_checksum_manifest "$extracted/$bundle_name"
    rm -rf -- "$temp_dir"
    printf 'client release artifact verified: %s\n' "$archive"
}

usage() {
    printf '%s\n' 'usage: verify-release-artifacts.sh crates OUTPUT_DIR' >&2
    printf '%s\n' '   or: verify-release-artifacts.sh client OUTPUT_DIR TAG TARGET CLI_BINARY MCP_BINARY' >&2
}

case "${1:-}" in
    crates)
        (( $# == 2 )) || { usage; exit 2; }
        verify_crates "$2"
        ;;
    client)
        (( $# == 6 )) || { usage; exit 2; }
        verify_client "$2" "$3" "$4" "$5" "$6"
        ;;
    *)
        usage
        exit 2
        ;;
esac
