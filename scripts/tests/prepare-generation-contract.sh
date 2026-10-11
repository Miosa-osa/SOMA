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

# The image is pulled from its registry unless the operator names another skopeo source, which is
# how an image that was built on this machine and never pushed becomes a Generation input.
mkdir -p "$WORK/bin" "$WORK/store"
cat >"$WORK/bin/skopeo" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >"$SKOPEO_LOG"
layout="${*: -1}"
layout="${layout#oci:}"
layout="${layout%:soma}"
: >"$layout/oci-layout"
STUB
chmod +x "$WORK/bin/skopeo"

expect_skopeo_source() {
    local expected="$1" setup="$2"
    (
        # export_oci reads the shared IMAGE and STORE_DIR inputs by design.
        # shellcheck disable=SC2030,SC2031
        export SKOPEO_LOG="$WORK/skopeo.log" PATH="$WORK/bin:$PATH"
        # shellcheck disable=SC2034
        STORE_DIR="$WORK/store"
        eval "$setup"
        export_oci >/dev/null
    )
    case "$(<"$WORK/skopeo.log")" in
        *" $expected oci:"*) ;;
        *)
            printf 'skopeo was not given source %s: %s\n' "$expected" "$(<"$WORK/skopeo.log")" >&2
            return 1
            ;;
    esac
}

expect_skopeo_source "docker://docker.io/library/node:22" 'IMAGE=node:22'
expect_skopeo_source "docker-daemon:soma-browser-default:1" \
    'IMAGE=soma-browser-default:1; export SOMA_IMAGE_SOURCE=docker-daemon:soma-browser-default:1'

printf 'prepare-generation shell contract passed\n'

# A memory ceiling builds the live-resize machine: the argument or the environment names it, the
# argument wins, and it must be above the boot memory because the difference is the hotplug region.
mkdir -p "$WORK/store" "$WORK/tools"

CEILING_MIB=""
unset SOMA_PREPARE_CEILING_MIB
parse_args node:22 "$WORK/store" "$WORK/tools" 2048 10240
[[ -z "$CEILING_MIB" ]] || { printf 'no ceiling was asked for but got %s\n' "$CEILING_MIB" >&2; exit 1; }

parse_args node:22 "$WORK/store" "$WORK/tools" 2048 10240 8192
[[ "$CEILING_MIB" == "8192" ]] || { printf 'ceiling argument lost: %s\n' "$CEILING_MIB" >&2; exit 1; }

SOMA_PREPARE_CEILING_MIB=4096 parse_args node:22 "$WORK/store" "$WORK/tools" 2048 10240
[[ "$CEILING_MIB" == "4096" ]] || { printf 'ceiling environment lost: %s\n' "$CEILING_MIB" >&2; exit 1; }

SOMA_PREPARE_CEILING_MIB=4096 parse_args node:22 "$WORK/store" "$WORK/tools" 2048 10240 16384
[[ "$CEILING_MIB" == "16384" ]] || { printf 'the argument must win over the environment: %s\n' "$CEILING_MIB" >&2; exit 1; }

for bad in 2048 1024 abc; do
    if (parse_args node:22 "$WORK/store" "$WORK/tools" 2048 10240 "$bad") >/dev/null 2>&1; then
        printf 'ceiling %s was accepted over 2048 MiB of boot memory\n' "$bad" >&2
        exit 1
    fi
done
