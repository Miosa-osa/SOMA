#!/usr/bin/env bash
#
# Prints a JSON array with one record per binary of a bundle, made from the binary's bytes and
# its own `--version` report.
#
#   describe-binaries.sh BUNDLE_DIR ARCHIVE BUNDLE_NAME RELEASE COMMIT ALLOW_DIRTY NAME...
#
# A report must name this release, the release profile, and this commit from a clean tree (or a
# dirty one when ALLOW_DIRTY is 1). The guest agent records no commit on purpose, so a report whose
# commit is `unknown` is accepted only with source `unknown`, and its record says so.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR
# shellcheck source=scripts/release/common.sh
source "${SCRIPT_DIR}/common.sh"

(( $# >= 7 )) ||
    fail 'usage: describe-binaries.sh BUNDLE_DIR ARCHIVE BUNDLE_NAME RELEASE COMMIT ALLOW_DIRTY NAME...'
bundle_dir="$1"
archive="$2"
bundle_name="$3"
release="$4"
commit="$5"
allow_dirty="$6"
shift 6
require_command jq

# The value of one fixed report key.
report_value() {
    printf '%s\n' "$1" | sed -n "s/^$2: //p" | head -n 1
}

for name in "$@"; do
    binary="$bundle_dir/$name"
    report="$("$binary" --version)" || fail "${name} --version did not succeed"
    program="${name%.exe}"
    [[ "$(printf '%s\n' "$report" | head -n 1)" == "${program} ${release} ("* ]] ||
        fail "${name} does not report itself as ${program} ${release}"
    [[ "$(report_value "$report" release)" == "$release" ]] || fail "${name} reports another release"
    [[ "$(report_value "$report" profile)" == release ]] || fail "${name} is not a release build"
    reported_commit="$(report_value "$report" commit)"
    source_state="$(report_value "$report" source)"
    if [[ "$reported_commit" == unknown ]]; then
        [[ "$source_state" == unknown ]] || fail "${name} reports a source without a commit"
        reports_commit=false
    else
        [[ "$reported_commit" == "$commit" ]] ||
            fail "${name} reports commit ${reported_commit}, expected ${commit}"
        if [[ "$source_state" != clean && ! ("$allow_dirty" == 1 && "$source_state" == dirty) ]]; then
            fail "${name} was built from a ${source_state} tree"
        fi
        reports_commit=true
    fi
    api="$(report_value "$report" api)"
    protocols="$(printf '%s\n' "$report" |
        sed -n 's/^protocol\.\([0-9A-Za-z_-]*\): \([0-9][0-9]*\)$/\1 \2/p' |
        jq -Rn '[inputs | split(" ") | {(.[0]): (.[1] | tonumber)}] | add // {}')"
    jq -n \
        --arg name "$name" \
        --arg archive "$archive" \
        --arg path "${bundle_name}/${name}" \
        --arg sha256 "$(sha256_of "$binary")" \
        --argjson size "$(size_of "$binary")" \
        --arg release "$release" \
        --arg commit "$commit" \
        --argjson reports_commit "$reports_commit" \
        --arg source "$source_state" \
        --arg target "$(report_value "$report" target)" \
        --arg api "$api" \
        --argjson protocols "$protocols" \
        '{name: $name, archive: $archive, path: $path, sha256: $sha256, size: $size,
          release: $release, commit: $commit, reports_commit: $reports_commit,
          source: $source, profile: "release", target: $target,
          api_version: (if $api == "" then null else $api end), protocols: $protocols}'
done | jq -s .
