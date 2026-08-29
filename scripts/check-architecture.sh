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

if (( failed != 0 )); then
    exit 1
fi

printf 'architecture checks passed\n'
