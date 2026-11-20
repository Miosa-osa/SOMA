#!/usr/bin/env bash
#
# The release-manifest half of scripts/verify-release-artifacts.sh. Source it after
# scripts/release/common.sh; it defines functions and changes nothing.

readonly RELEASE_MANIFEST_SCHEMA="soma.release-manifest.v1"

# Checks one binary record against the binary it describes and where it says that binary is.
verify_binary_record() {
    local record="$1"
    local binary="$2"
    local archive="$3"
    local path="$4"
    local release="$5"

    [[ -n "$record" ]] || fail "no binary record for ${path}"
    jq -e --arg archive "$archive" --arg path "$path" --arg release "$release" \
        --arg sha256 "$(sha256_of "$binary")" --argjson size "$(size_of "$binary")" '
        .archive == $archive and .path == $path and .release == $release
        and .sha256 == $sha256 and .size == $size and .profile == "release"
        and (.commit | test("^([0-9a-f]{40}|[0-9a-f]{64})$"))
        and (.reports_commit | type == "boolean")
        and (.source | IN("clean", "dirty", "unknown"))
        and (.target | type == "string" and length > 0)
        and (.api_version == null or (.api_version | test("^[0-9]+[.][0-9]+$")))
        and (.protocols | type == "object" and all(.[]; type == "number"))
        ' <<< "$record" >/dev/null ||
        fail "the record of ${path} does not match the binary: ${record}"
}

verify_release() {
    local directory="$1"
    local release_tag="$2"
    local manifest="$directory/soma-${release_tag}-manifest.json"
    local listed actual name archive extracted record

    [[ -d "$directory" ]] || fail "release directory is missing: ${directory}"
    require_command jq
    require_command tar
    verify_checksum_manifest "$directory"
    [[ -f "$manifest" ]] || fail "release manifest is missing: ${manifest}"
    jq -e --arg schema "$RELEASE_MANIFEST_SCHEMA" --arg tag "$release_tag" '
        .schema == $schema and .tag == $tag and ("v" + .release) == $tag
        and .prerelease == (.release | contains("-"))
        and (.commit | test("^([0-9a-f]{40}|[0-9a-f]{64})$"))
        and (.binaries | length > 0)
        ' "$manifest" >/dev/null ||
        fail "release manifest does not describe ${release_tag}"
    listed="$(jq -r '.artifacts[].file' "$manifest" | LC_ALL=C sort)"
    actual="$(find "$directory" -mindepth 1 -maxdepth 1 -type f ! -name SHA256SUMS \
        ! -name "${manifest##*/}" -exec basename {} \; | LC_ALL=C sort)"
    [[ "$listed" == "$actual" ]] || fail 'the manifest does not list exactly the shipped artifacts'
    while IFS= read -r name; do
        jq -e --arg file "$name" --arg sha256 "$(sha256_of "$directory/$name")" \
            --argjson size "$(size_of "$directory/$name")" \
            '.artifacts[] | select(.file == $file) | .sha256 == $sha256 and .size == $size' \
            "$manifest" >/dev/null || fail "the manifest digest of ${name} is wrong"
        if [[ "$name" != "soma-${release_tag}-source.tar.gz" ]]; then
            jq -e --arg file "$name" 'any(.binaries[]; .archive == $file)' "$manifest" >/dev/null ||
                fail "no binary record names the archive ${name}"
        fi
    done <<< "$listed"
    grep -Fxq "soma-${release_tag}-source.tar.gz" <<< "$actual" ||
        fail 'the release has no source archive'
    extracted="${VERIFY_WORK_DIR:?}/release"
    mkdir -p "$extracted"
    while IFS= read -r archive; do
        mkdir -p "$extracted/$archive"
        tar -xzf "$directory/$archive" -C "$extracted/$archive"
    done < <(jq -r '[.binaries[].archive] | unique | .[]' "$manifest")
    while IFS= read -r record; do
        archive="$(jq -r .archive <<< "$record")"
        verify_binary_record "$record" "$extracted/$archive/$(jq -r .path <<< "$record")" \
            "$archive" "$(jq -r .path <<< "$record")" "${release_tag#v}"
        jq -e --slurpfile manifest "$manifest" '.commit == $manifest[0].commit' <<< "$record" \
            >/dev/null || fail "$(jq -r .path <<< "$record") names another commit"
    done < <(jq -c '.binaries[]' "$manifest")
    jq -e '.api_version == ([.binaries[] | select(.name == "soma-api") | .api_version] | first)' \
        "$manifest" >/dev/null || fail 'the manifest API version is not the one soma-api reports'
    printf 'release verified: %s (%s binaries)\n' "$directory" "$(jq '.binaries | length' "$manifest")"
}
