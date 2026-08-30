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

printf 'prepare-generation shell contract passed\n'
