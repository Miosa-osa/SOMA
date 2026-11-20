#!/usr/bin/env bash
#
# Shared by the release packagers and the release verifier: failure, required tools, SHA-256,
# and the checksum manifest rule. Source it; it defines functions and changes nothing.

fail() {
    printf '%s\n' "$1" >&2
    exit 1
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        fail "required command not found: $1"
    fi
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -- "$1" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -- "$1" | awk '{ print $1 }'
    else
        fail 'sha256sum or shasum is required'
    fi
}

size_of() {
    wc -c < "$1" | tr -d ' '
}

# Writes DIRECTORY/NAME covering exactly the named files of DIRECTORY, in the order given.
write_checksum_manifest() {
    local directory="$1"
    local manifest_name="$2"
    shift 2
    local file

    : > "$directory/$manifest_name"
    for file in "$@"; do
        printf '%s  %s\n' "$(sha256_of "$directory/$file")" "$file" >> "$directory/$manifest_name"
    done
}

# Prints the file names a checksum manifest lists, refusing malformed or unsafe records.
checksum_manifest_names() {
    local manifest="$1"
    local digest filename extra

    while read -r digest filename extra; do
        if [[ ! "$digest" =~ ^[0-9a-f]{64}$ || -z "$filename" || -n "${extra:-}" ]]; then
            fail "invalid checksum record in ${manifest}"
        fi
        # GNU checksum manifests prefix binary-mode file names with one `*` marker.
        if [[ "$filename" == \** ]]; then
            filename="${filename#\*}"
        fi
        if [[ -z "$filename" || "$filename" == -* || "$filename" == */* ||
            "$filename" == *\\* ]]; then
            fail "unsafe checksum path in ${manifest}: ${filename}"
        fi
        printf '%s\n' "$filename"
    done < "$manifest"
}

# Checks that DIRECTORY/NAME covers exactly the regular files of DIRECTORY and that every
# digest matches. The manifest itself is the one file it does not list.
verify_checksum_manifest() {
    local directory="$1"
    local manifest_name="${2:-SHA256SUMS}"
    local manifest="$directory/$manifest_name"
    local listed actual unexpected name digest

    [[ -f "$manifest" ]] || fail "checksum manifest is missing: ${manifest}"
    unexpected="$(find "$directory" -mindepth 1 -maxdepth 1 ! -type f -print -quit)"
    [[ -z "$unexpected" ]] || fail "release directory contains a non-regular entry: ${unexpected}"
    listed="$(checksum_manifest_names "$manifest" | LC_ALL=C sort)"
    [[ -n "$listed" ]] || fail "checksum manifest is empty: ${manifest}"
    if [[ -n "$(printf '%s\n' "$listed" | uniq -d)" ]]; then
        fail "checksum manifest contains duplicate paths: ${manifest}"
    fi
    actual="$(find "$directory" -mindepth 1 -maxdepth 1 -type f ! -name "$manifest_name" \
        -exec basename {} \; | LC_ALL=C sort)"
    if [[ "$listed" != "$actual" ]]; then
        diff -u <(printf '%s\n' "$listed") <(printf '%s\n' "$actual") >&2 || true
        fail "checksum manifest does not cover exactly the shipped files in ${directory}"
    fi
    while read -r digest name; do
        name="${name#\*}"
        if [[ "$(sha256_of "$directory/$name")" != "$digest" ]]; then
            fail "checksum mismatch for ${directory}/${name}"
        fi
    done < "$manifest"
}
