#!/usr/bin/env bash
#
# Holds still the parts of the live memory resize check that decide pass from fail.
#
# The check itself needs a KVM host. These do not: they feed the envelope reader and the shell
# helpers canned command line output, so a change that would make a stage read the wrong field
# or accept a refusal as a success fails here first.

set -euo pipefail

TEST_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
readonly TEST_REPO_ROOT

FIXTURE_WORK="$(mktemp -d)"
readonly FIXTURE_WORK
trap 'rm -rf -- "$FIXTURE_WORK"' EXIT

# shellcheck disable=SC1091
source "$TEST_REPO_ROOT/scripts/live-memory-resize.sh"

fail() { printf '%s\n' "$1" >&2; exit 1; }

envelope() {
    python3 - "$@" <<'PY'
import base64, json, sys
kind, status = sys.argv[1], sys.argv[2]
result = {}
if kind == "launch":
    result = {"instance_id": sys.argv[3]}
elif kind == "exec":
    text = sys.argv[3].encode()
    result = {"execution": {"exited": {"code": int(sys.argv[4])}},
              "stdout": {"encoding": "base64", "data": base64.b64encode(text).decode()}}
elif kind == "resize":
    state, plugged, total = sys.argv[3], int(sys.argv[4]), int(sys.argv[5])
    result = {"cpu": {"state": "unchanged"},
              "memory": {"state": state, "plugged_mb": plugged, "total_mb": total,
                         "booted_mb": 1024, "ceiling_mb": 8192}}
print(json.dumps({"schema": "soma.cli.v1", "status": status, "result": result}))
PY
}

expect_judge() {
    local expected="$1" want="$2" actual status=0
    shift 2
    actual="$(python3 "$JUDGE" "$@" 2>&1)" || status=$?
    if [[ "$expected" == pass && "$status" -ne 0 ]]; then fail "should pass, got: $actual ($*)"; fi
    if [[ "$expected" != pass && "$status" -eq 0 ]]; then fail "should fail, got: $actual ($*)"; fi
    if [[ "$expected" == pass && "$actual" != "$want" ]]; then
        fail "got '$actual', expected '$want' ($*)"
    fi
    return 0
}

id32="0123456789abcdef0123456789abcdef"
envelope launch ok "$id32" >"$FIXTURE_WORK/launch.json"
envelope launch ok "not-an-id" >"$FIXTURE_WORK/launch-bad.json"
envelope launch error "$id32" >"$FIXTURE_WORK/launch-error.json"
expect_judge pass "$id32" instance "$FIXTURE_WORK/launch.json"
expect_judge fail "" instance "$FIXTURE_WORK/launch-bad.json"
expect_judge fail "" instance "$FIXTURE_WORK/launch-error.json"
expect_judge fail "" instance "$FIXTURE_WORK/missing.json"

envelope exec ok "MemTotal:        3921320 kB" 0 >"$FIXTURE_WORK/exec.json"
envelope exec ok "boom" 3 >"$FIXTURE_WORK/exec-failed.json"
expect_judge pass "MemTotal:        3921320 kB" stdout "$FIXTURE_WORK/exec.json"
expect_judge fail "" stdout "$FIXTURE_WORK/exec-failed.json"

envelope resize ok applied_live 2048 3072 >"$FIXTURE_WORK/resize.json"
envelope resize ok partial 512 1536 >"$FIXTURE_WORK/partial.json"
envelope resize error applied_live 2048 3072 >"$FIXTURE_WORK/resize-error.json"
expect_judge pass "applied_live 2048 3072 1024 8192" memory "$FIXTURE_WORK/resize.json"
expect_judge pass "partial 512 1536 1024 8192" memory "$FIXTURE_WORK/partial.json"
expect_judge fail "" memory "$FIXTURE_WORK/resize-error.json"
expect_judge fail "" memory "$FIXTURE_WORK/launch.json"

printf 'MemTotal:  100 kB\nMemFree:   50 kB\n' | python3 "$JUDGE" meminfo MemTotal | grep -qx 100 \
    || fail "meminfo did not read MemTotal"
if printf 'MemFree: 50 kB\n' | python3 "$JUDGE" meminfo MemTotal >/dev/null; then
    fail "meminfo accepted a file with no MemTotal"
fi

# The helpers, with the command line replaced by what it would have printed.
WORK="$FIXTURE_WORK"
INSTANCE="$id32"
MEMINFO_KIB=3000000
soma() {
    case "$2" in
        exec) envelope exec ok "MemTotal:  $MEMINFO_KIB kB" 0 ;;
        resize) envelope resize ok applied_live "$(( ${6:-1024} - 1024 ))" "${6:-1024}" ;;
    esac
}

[[ "$(guest_memtotal_kib)" == "$MEMINFO_KIB" ]] || fail "guest_memtotal_kib read the wrong number"
wait_memtotal_at_least 2999999 2 || fail "an already satisfied wait did not return at once"
wait_memtotal_at_most 3000001 2 || fail "an already satisfied wait did not return at once"
if wait_memtotal_at_least 4000000 1; then fail "a wait for memory that never came succeeded"; fi
if wait_memtotal_at_most 1000000 1; then fail "a wait for memory to shrink succeeded without it"; fi

[[ "$(resize_to 3072)" == "applied_live 2048 3072 1024 8192" ]] || fail "default resize verb output"
RESIZE_CMD="printf '%s' '{id} {mib}' >$FIXTURE_WORK/template; cat $FIXTURE_WORK/resize.json"
resize_to 4096 >/dev/null || fail "a templated resize command was not run"
[[ "$(cat "$FIXTURE_WORK/template")" == "$id32 4096" ]] || fail "the template placeholders were not filled"

if [[ "$(uname -s)" == Linux ]]; then
    (( $(host_rss_kib "$$") > 0 )) || fail "host_rss_kib read no memory for a live process"
fi
[[ "$(host_rss_kib 999999999)" == 0 ]] || fail "host_rss_kib invented memory for a missing process"

# A plug size the device cannot take in whole blocks, or a touch that cannot fit, is refused
# before any stage runs.
for args in "--plug-mib 3" "--plug-mib 0" "--plug-mib 100 --touch-mib 100" "--bogus"; do
    # shellcheck disable=SC2086
    if (main $args) >/dev/null 2>&1; then fail "arguments '$args' were accepted"; fi
done

printf 'live memory resize contract: ok\n'
