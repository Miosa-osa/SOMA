#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly MAX_SOURCE_LINES=300
DASH_PATTERN="$(printf '\342\200\223|\342\200\224')"
readonly DASH_PATTERN

cd "${REPO_ROOT}"

failed=0

while IFS= read -r -d '' dumping_ground; do
    printf '%s uses a prohibited generic module name\n' "$dumping_ground" >&2
    failed=1
done < <(
    find crates -type f \( \
        -name 'utils.rs' -o \
        -name 'helpers.rs' -o \
        -name 'common.rs' -o \
        -name 'manager.rs' -o \
        -name 'core.rs' \
    \) -print0
)

is_exempt_path() {
    case "$1" in
        ./.git/*|*/target/*|*/generated/*|*/fixtures/*|*/third_party/*|*/vendor/*)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

is_source_file() {
    case "$1" in
        *.rs|*.proto|*.sh|*.bash|*.py|*.c|*.h|*.cc|*.cpp|*.S|*.toml|*.yml|*.yaml)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

while IFS= read -r -d '' file; do
    if is_exempt_path "$file"; then
        continue
    fi

    if [[ "$file" != "./Cargo.lock" ]] && is_source_file "$file"; then
        line_count="$(awk 'END { print NR }' "$file")"
        if (( line_count > MAX_SOURCE_LINES )); then
            printf '%s has %s lines; authored source files must have at most %s\n' \
                "$file" "$line_count" "$MAX_SOURCE_LINES" >&2
            failed=1
        fi
    fi

    if [[ -s "$file" ]] && LC_ALL=C grep -Iq . "$file"; then
        if LC_ALL=C grep -n -E "$DASH_PATTERN" "$file"; then
            printf '%s contains an en dash or em dash\n' "$file" >&2
            failed=1
        fi
    fi
done < <(find . -type f -print0)

while IFS= read -r duplicate_adr; do
    printf 'ADR number %s is used by more than one decision record\n' "$duplicate_adr" >&2
    failed=1
done < <(
    find docs/adr -type f -name '[0-9][0-9][0-9][0-9]-*.md' -printf '%f\n' \
        | cut -c1-4 \
        | sort \
        | uniq -d
)

readonly CLAIM_LEDGER=docs/claim-ledger.md

if [[ ! -f "${CLAIM_LEDGER}" ]]; then
    printf '%s is missing; every capability claim must have a ledger row\n' "${CLAIM_LEDGER}" >&2
    failed=1
else
    while IFS= read -r status; do
        printf '%s has a status cell that is not one of the five status terms: %s\n' \
            "${CLAIM_LEDGER}" "$status" >&2
        failed=1
    done < <(
        awk -F '|' '
            /^\|/ && NF >= 4 {
                status = $3
                gsub(/^[ \t]+|[ \t]+$/, "", status)
                if (status == "Status" || status ~ /^-+$/ || status == "") {
                    next
                }
                if (status !~ /^(Designed|Component-tested|Live-proved|Integrated|Production-admitted)([ ;,]|$)/) {
                    print status
                }
            }
        ' "${CLAIM_LEDGER}"
    )

    while IFS= read -r target; do
        printf '%s links to %s, which does not exist\n' "${CLAIM_LEDGER}" "$target" >&2
        failed=1
    done < <(
        grep -o '](\([^)#]*\)[^)]*)' "${CLAIM_LEDGER}" \
            | sed 's/^](//; s/[)#].*$//' \
            | grep -v '^https\{0,1\}:' \
            | sort -u \
            | while IFS= read -r relative; do
                if [[ ! -e "docs/${relative}" ]]; then
                    printf '%s\n' "$relative"
                fi
            done
    )
fi

if (( failed != 0 )); then
    exit 1
fi

printf 'architecture checks passed\n'
