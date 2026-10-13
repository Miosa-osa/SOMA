#!/usr/bin/env bash
#
# Runs the three ADR 0053 Step A persistence gates against a real host and records evidence.
#
# A gate is green only when it passes on the shipped binary through the public API, so this
# script does not implement the sandbox calls itself: it drives them through an adapter the
# caller supplies, and it judges the outcome. Keeping the calls in an adapter is what makes the
# judgement testable without a host, which is what scripts/tests/persistence-gate-contract.sh
# does, and it is why this file and that one are written as a pair.
#
# The three gates are:
#   1. write a file, stop, start on the same host, the file is present and the process is gone.
#   2. write a file, stop, reboot the host, start, the file is present.
#   3. a running sandbox survives a soma-api restart with its file present (needs SOMA PR #67).
#
# Usage:
#   persistence-gate.sh --adapter PATH --sandbox ID --runtime-digest SHA [--gate N] [--evidence DIR]
#
# The adapter is any executable called as:
#   ADAPTER write  SANDBOX PATH CONTENT   # exits 0 when the file was written
#   ADAPTER read   SANDBOX PATH           # prints the file content on stdout
#   ADAPTER exec   SANDBOX COMMAND        # prints the command output on stdout
#   ADAPTER stop   SANDBOX                # stops the sandbox, keeping its disk
#   ADAPTER start  SANDBOX                # starts the sandbox again
#   ADAPTER reboot HOST                   # reboots the host and waits for it back (gate 2 only)
#   ADAPTER api-restart HOST              # restarts soma-api and waits for it back (gate 3 only)
# Every adapter call must be idempotent enough to be run once; the script does not retry.

set -euo pipefail

ADAPTER=""
SANDBOX=""
RUNTIME_DIGEST=""
HOST=""
EVIDENCE_DIR=""
GATE=0

usage() {
    sed -n '3,32p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

die() {
    printf 'persistence-gate: %s\n' "$1" >&2
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --adapter) ADAPTER="${2:-}"; shift 2 ;;
        --sandbox) SANDBOX="${2:-}"; shift 2 ;;
        --runtime-digest) RUNTIME_DIGEST="${2:-}"; shift 2 ;;
        --host) HOST="${2:-}"; shift 2 ;;
        --evidence) EVIDENCE_DIR="${2:-}"; shift 2 ;;
        --gate) GATE="${2:-}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done

[[ -n "$ADAPTER" ]] || die "--adapter is required"
[[ -x "$ADAPTER" ]] || die "adapter is not executable: $ADAPTER"
[[ -n "$SANDBOX" ]] || die "--sandbox is required"
[[ -n "$RUNTIME_DIGEST" ]] || die "--runtime-digest is required"
case "$GATE" in
    0|1|2|3) ;;
    *) die "--gate must be 1, 2, or 3" ;;
esac

# Gate 2 reboots the host and gate 3 restarts soma-api, so both need the host name.
if [[ "$GATE" == 2 || "$GATE" == 3 || "$GATE" == 0 ]]; then
    [[ -n "$HOST" ]] || die "--host is required for gate 2 and gate 3"
fi

MARKER="/workspace/persistence-gate-marker"
PAYLOAD="step-a-$(date -u +%Y%m%dT%H%M%SZ)"

RESULT="pass"
FIRST_FAILURE=""

fail() {
    RESULT="fail"
    [[ -n "$FIRST_FAILURE" ]] || FIRST_FAILURE="$1"
    printf 'FAIL: %s\n' "$1" >&2
}

# One step, echoed as a command line so the evidence can be replayed.
step() {
    printf '+ %s\n' "$*"
    "$@"
}

digest_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    else
        shasum -a 256 | awk '{print $1}'
    fi
}

# Gate 1: write, stop, start on the same host, file present and process gone.
gate_1() {
    printf '\n==> gate 1: write, stop, start, same host\n'
    step "$ADAPTER" write "$SANDBOX" "$MARKER" "$PAYLOAD" || { fail "gate 1 write"; return; }
    step "$ADAPTER" exec "$SANDBOX" "sleep 600 & echo \$! > /workspace/persistence-gate-pid" \
        || fail "gate 1 background process"
    step "$ADAPTER" stop "$SANDBOX" || { fail "gate 1 stop"; return; }
    step "$ADAPTER" start "$SANDBOX" || { fail "gate 1 start"; return; }
    local read_back
    read_back="$("$ADAPTER" read "$SANDBOX" "$MARKER" 2>/dev/null || true)"
    if [[ "$read_back" != "$PAYLOAD" ]]; then
        fail "gate 1 file content changed across stop and start"
    fi
    local survivors
    survivors="$("$ADAPTER" exec "$SANDBOX" "pgrep -f 'sleep 600' || true" 2>/dev/null || true)"
    if [[ -n "$survivors" ]]; then
        fail "gate 1 a process survived the stop"
    fi
}

# Gate 2: write, stop, reboot the host, start, file present.
gate_2() {
    printf '\n==> gate 2: write, stop, host reboot, start\n'
    step "$ADAPTER" write "$SANDBOX" "$MARKER" "$PAYLOAD" || { fail "gate 2 write"; return; }
    step "$ADAPTER" stop "$SANDBOX" || { fail "gate 2 stop"; return; }
    step "$ADAPTER" reboot "$HOST" || { fail "gate 2 reboot"; return; }
    step "$ADAPTER" start "$SANDBOX" || { fail "gate 2 start"; return; }
    local read_back
    read_back="$("$ADAPTER" read "$SANDBOX" "$MARKER" 2>/dev/null || true)"
    if [[ "$read_back" != "$PAYLOAD" ]]; then
        fail "gate 2 the sidecar rebuild did not recover the volume"
    fi
}

# Gate 3: a running sandbox survives a soma-api restart with its file present.
gate_3() {
    printf '\n==> gate 3: soma-api restart mid-run\n'
    step "$ADAPTER" write "$SANDBOX" "$MARKER" "$PAYLOAD" || { fail "gate 3 write"; return; }
    step "$ADAPTER" api-restart "$HOST" || { fail "gate 3 api-restart"; return; }
    local answer
    answer="$("$ADAPTER" exec "$SANDBOX" 'echo alive' 2>/dev/null || true)"
    if [[ "$answer" != *alive* ]]; then
        fail "gate 3 exec did not answer after the api restart"
    fi
    local read_back
    read_back="$("$ADAPTER" read "$SANDBOX" "$MARKER" 2>/dev/null || true)"
    if [[ "$read_back" != "$PAYLOAD" ]]; then
        fail "gate 3 the file did not survive the api restart"
    fi
}

case "$GATE" in
    1) gate_1 ;;
    2) gate_2 ;;
    3) gate_3 ;;
    0) gate_1; gate_2; gate_3 ;;
esac

if [[ -n "$EVIDENCE_DIR" ]]; then
    mkdir -p "$EVIDENCE_DIR"
    printf 'gate=%s\nresult=%s\nsandbox=%s\nhost=%s\nruntime_digest=%s\nfirst_failure=%s\nadapter_digest=%s\n' \
        "$GATE" "$RESULT" "$SANDBOX" "$HOST" "$RUNTIME_DIGEST" "$FIRST_FAILURE" \
        "$(digest_of <"$ADAPTER")" >"$EVIDENCE_DIR/persistence-gate-$GATE.txt"
fi

printf '\n==> gate %s: %s\n' "$GATE" "$RESULT"
[[ "$RESULT" == pass ]] || exit 1
