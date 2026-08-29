#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly SEMVER_PATTERN='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'

cd "$REPO_ROOT"

if [[ ! -f VERSION ]]; then
    printf 'VERSION is missing\n' >&2
    exit 1
fi

source_version="$(tr -d '\r\n' < VERSION)"
if [[ ! "$source_version" =~ $SEMVER_PATTERN ]]; then
    printf 'VERSION is not valid Semantic Versioning: %s\n' "${source_version:-empty}" >&2
    exit 1
fi

if [[ "$(wc -l < VERSION | tr -d ' ')" != "1" ]]; then
    printf 'VERSION must contain exactly one line\n' >&2
    exit 1
fi

workspace_version="$(sed -n -E 's/^version = "([^"]+)"$/\1/p' Cargo.toml)"
if [[ -z "$workspace_version" ]]; then
    printf 'Cargo workspace package version is missing\n' >&2
    exit 1
fi

if [[ "$source_version" != "$workspace_version" ]]; then
    printf 'VERSION %s does not match Cargo workspace version %s\n' \
        "$source_version" "$workspace_version" >&2
    exit 1
fi

printf 'version contract passed: %s\n' "$source_version"
