#!/usr/bin/env bash

set -euo pipefail
shopt -s nullglob

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly USES_PATTERN='uses:[[:space:]]*([^[:space:]#]+)'
readonly ACTION_PATTERN='^[^@]+@[0-9a-f]{40}$'
readonly CONTAINER_PATTERN='^docker://[^@]+@sha256:[0-9a-f]{64}$'

cd "${REPO_ROOT}"

workflow_files=(.github/workflows/*.yml .github/workflows/*.yaml)
if (( ${#workflow_files[@]} == 0 )); then
    printf 'no GitHub Actions workflows found\n' >&2
    exit 1
fi

failed=0
checkout_count=0
credential_count=0

for workflow in "${workflow_files[@]}"; do
    if ! grep -q '^permissions:' "$workflow"; then
        printf '%s must declare top-level permissions\n' "$workflow" >&2
        failed=1
    fi

    if grep -n -E '^[[:space:]]*pull_request_target:|permissions:[[:space:]]*write-all|persist-credentials:[[:space:]]*true|secrets:[[:space:]]*inherit' "$workflow"; then
        printf '%s contains a forbidden workflow privilege pattern\n' "$workflow" >&2
        failed=1
    fi

    if grep -n -E '(curl|wget)[^|]*\|[[:space:]]*(bash|sh)' "$workflow"; then
        printf '%s pipes a network response into a shell\n' "$workflow" >&2
        failed=1
    fi

    line_number=0
    while IFS= read -r line || [[ -n "$line" ]]; do
        line_number=$((line_number + 1))

        if [[ "$line" =~ $USES_PATTERN ]]; then
            target="${BASH_REMATCH[1]}"
            target="${target#\"}"
            target="${target%\"}"
            target="${target#\'}"
            target="${target%\'}"

            if [[ "$target" == ./* ]]; then
                continue
            fi

            if [[ "$target" == docker://* ]]; then
                if [[ ! "$target" =~ $CONTAINER_PATTERN ]]; then
                    printf '%s:%s container reference is not pinned by digest: %s\n' \
                        "$workflow" "$line_number" "$target" >&2
                    failed=1
                fi
            elif [[ ! "$target" =~ $ACTION_PATTERN ]]; then
                printf '%s:%s action is not pinned to a 40-character commit SHA: %s\n' \
                    "$workflow" "$line_number" "$target" >&2
                failed=1
            fi

            if [[ "$target" == actions/checkout@* ]]; then
                checkout_count=$((checkout_count + 1))
            fi
        fi

        if [[ "$line" =~ persist-credentials:[[:space:]]*false ]]; then
            credential_count=$((credential_count + 1))
        fi
    done < "$workflow"
done

if (( checkout_count != credential_count )); then
    printf 'each checkout step must set persist-credentials to false: %s checkout steps, %s settings\n' \
        "$checkout_count" "$credential_count" >&2
    failed=1
fi

if (( failed != 0 )); then
    exit 1
fi

printf 'workflow policy checks passed\n'
