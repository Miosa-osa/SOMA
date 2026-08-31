#!/usr/bin/env bash

set -euo pipefail

TEST_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly TEST_REPO_ROOT

# shellcheck disable=SC1091
source "$TEST_REPO_ROOT/scripts/prepare-generation.sh"

expect_source() {
    IMAGE="$1"
    local actual
    actual="$(registry_reference)"
    if [[ "$actual" != "$2" ]]; then
        printf 'source mismatch for %s: got %s, expected %s\n' "$1" "$actual" "$2" >&2
        return 1
    fi
}

expect_source "node:22" "docker.io/library/node:22"
expect_source "acme/agent:1" "docker.io/acme/agent:1"
expect_source "ghcr.io/acme/agent:1" "ghcr.io/acme/agent:1"
expect_source "localhost:5000/acme/agent:1" "localhost:5000/acme/agent:1"

IMAGE="acme/a-b:c"
first="$(reference_key)"
# `reference_key` reads the shared IMAGE input by design.
# shellcheck disable=SC2034
IMAGE="acme/a/b:c"
second="$(reference_key)"
if [[ "$first" == "$second" ]]; then
    printf 'distinct references produced the same key\n' >&2
    exit 1
fi

# An entry with no snapshot cold boots, roughly fifteen times slower, and nothing at launch
# says so. The script must refuse to end quietly on one.
WORK="$(mktemp -d)"
trap 'rm -rf -- "$WORK"' EXIT
MEM_MIB=1024

expect_boot_report() {
    local entry="$1" expected_status="$2" expected_text="$3" allow="$4"
    local status=0 output
    output="$(SOMA_ALLOW_COLD_BOOT_ENTRY="$allow" report_boot_path "$entry" 2>&1)" || status=$?
    if (( status != expected_status )); then
        printf 'boot report for %s exited %s, expected %s\n' \
            "$entry" "$status" "$expected_status" >&2
        return 1
    fi
    case "$output" in
        *"$expected_text"*) ;;
        *)
            printf 'boot report for %s did not mention %s\n' "$entry" "$expected_text" >&2
            printf '%s\n' "$output" >&2
            return 1
            ;;
    esac
}

mkdir -p "$WORK/cold"
expect_boot_report "$WORK/cold" 3 "COLD BOOT ONLY" 0
expect_boot_report "$WORK/cold" 3 "capture_snapshot" 0
expect_boot_report "$WORK/cold" 0 "accepted a cold boot only entry" 1

mkdir -p "$WORK/warm/snapshot"
: >"$WORK/warm/snapshot/state.somasnap"
expect_boot_report "$WORK/warm" 0 "RESTORE from" 0

printf 'prepare-generation shell contract passed\n'
