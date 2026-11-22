#!/usr/bin/env bash
#
# The suspension proof (ADR 0034) with one command: a running sandbox keeps its processes, its
# memory and its files across a suspend and a resume.
#
# It starts a private soma-api on its own loopback port and state root, so a soma-api already
# serving this host is not touched. It creates one sandbox from a Generation compiled with this
# tree's guest agent, starts a counter process and writes a file and a tmpfs value, suspends the
# sandbox into a sealed Suspension, resumes it as a new sandbox, and checks that the same counter
# process is still counting, that the file and the tmpfs value survived, and that the Suspension is
# gone once resumed. Every request and response is kept under <work>/results.
#
# Run it as root from the repository root on the KVM host, after build-soma.sh,
# `cargo build --release -p soma-api` and scripts/prepare-generation.sh for the reference.
#
# Usage:
#   scripts/suspend-proof.sh --store DIR --reference REF --work DIR [--port PORT]
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
STORE="" REFERENCE="" WORK="" PORT=18931
while (( $# > 0 )); do
    case "$1" in
        --store) STORE="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        --work) WORK="$2"; shift 2 ;;
        --port) PORT="$2"; shift 2 ;;
        *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
die() { printf 'suspend-proof: %s\n' "$1" >&2; exit 1; }
[[ -n "$STORE" && -n "$REFERENCE" && -n "$WORK" ]] || die "--store, --reference and --work are required"
[[ "$(id -u)" == 0 ]] || die "run as root: the runtime needs /dev/kvm"
[[ -x "$REPO_ROOT/target/release/soma-api" ]] || die "target/release/soma-api is missing; build it first"
mkdir -p "$WORK"
WORK="$(cd "$WORK" && pwd -P)"
[[ ! -e "$WORK/results" ]] || die "$WORK/results already exists; use a new --work directory"
OUT="$WORK/results"
mkdir -p "$OUT" "$WORK/state" "$WORK/heads"
export SOMA_PROOF_API="127.0.0.1:$PORT" SOMA_PROOF_TENANT=suspend-proof
# shellcheck source=scripts/desktop-proof/api.sh
source "$REPO_ROOT/scripts/desktop-proof/api.sh"
log() { printf '\n==> %s\n' "$1" | tee -a "$OUT/calls.log"; }
record() { printf '%s\t%s\n' "$1" "$2" | tee -a "$OUT/measurements.tsv"; }
failures=0
check() {
    if [[ "$2" == "$3" ]]; then
        printf 'ok    %s\n' "$1" | tee -a "$OUT/checks.txt"
    else
        printf 'FAIL  %s: expected %s, saw %s\n' "$1" "$3" "$2" | tee -a "$OUT/checks.txt"
        failures=$((failures + 1))
    fi
}

log "starting a private soma-api on $SOMA_PROOF_API"
SOMA_GENERATION_STORE="$STORE" SOMA_HEAD_DIR="$WORK/heads" \
    "$REPO_ROOT/target/release/soma-api" --listen "$SOMA_PROOF_API" --backend kvm \
    --state-root "$WORK/state" --workers 8 >"$OUT/soma-api.log" 2>&1 &
API_PID=$!
INSTANCE=""
RESUMED=""
cleanup() {
    for sandbox in "$INSTANCE" "$RESUMED"; do
        [[ -n "$sandbox" ]] || continue
        curl -sS -m 120 -X DELETE "http://$SOMA_PROOF_API/v1/sandboxes/$sandbox" \
            -H "x-soma-tenant: $SOMA_PROOF_TENANT" >/dev/null 2>&1 || true
    done
    kill "$API_PID" 2>/dev/null || true
    wait "$API_PID" 2>/dev/null || true
}
trap cleanup EXIT
for _ in $(seq 1 100); do
    curl -sS -m 2 -o /dev/null "http://$SOMA_PROOF_API/v1/version" \
        -H "x-soma-tenant: $SOMA_PROOF_TENANT" 2>/dev/null && break
    sleep 0.1
done

log "creating one sandbox from $REFERENCE"
api create POST /v1/sandboxes "$(printf '{"image":"%s"}' "$REFERENCE")"
[[ "$STATUS" == 201 ]] || die "create answered $STATUS: $(cat "$LAST")"
INSTANCE="$(field 'd["result"]["instance_id"]')"
record create_s "$SECONDS_TAKEN"

log "starting a counter, writing a file and a tmpfs value"
guest start sh -c 'mkdir -p /run/proof && echo "$$-$RANDOM-$RANDOM" > /run/proof/tmpfs-value
cat /run/proof/tmpfs-value > /root/disk-value && sync
setsid nohup sh -c "i=0; while :; do i=\$((i+1)); echo \$i > /run/proof/counter; sleep 0.2; done" >/dev/null 2>&1 &
echo $! > /run/proof/counter.pid; sleep 1; cat /run/proof/counter.pid'
check "the counter started" "$GUEST_EXIT" 0
COUNTER_PID="$(tr -d '[:space:]' <"$GUEST_OUT")"
guest before sh -c 'cat /run/proof/tmpfs-value; cat /run/proof/counter; cat /proc/uptime'
TMPFS_VALUE="$(sed -n 1p "$GUEST_OUT")"
COUNT_BEFORE="$(sed -n 2p "$GUEST_OUT")"

log "suspending"
api suspend POST "/v1/sandboxes/$INSTANCE/suspend" "$(printf '{"image":"%s"}' "$REFERENCE")"
check "suspend answered 200" "$STATUS" 200
[[ "$STATUS" == 200 ]] || die "suspend answered $STATUS: $(cat "$LAST")"
SUSPENSION="$(field 'd["result"]["suspension_id"]')"
record suspend_s "$SECONDS_TAKEN"
record sealed_bytes "$(field 'd["result"]["sealed_bytes"]')"
check "the receipt says Suspended" "$(field 'd["receipt"]["terminal_status"]')" suspended
SUSPENDED="$INSTANCE"
INSTANCE=""
api old-gone POST "/v1/sandboxes/$SUSPENDED/commands" '{"executable":"true"}'
check "the suspended sandbox runs nothing" "$([[ "$STATUS" =~ ^4 ]] && echo refused || echo "$STATUS")" refused
find "$WORK/state/suspensions" -maxdepth 3 -name '*.raw' >"$OUT/plaintext-after-seal.txt"
check "no plaintext member is left after sealing" "$(wc -l <"$OUT/plaintext-after-seal.txt")" 0

log "resuming as a new sandbox"
api resume POST "/v1/suspensions/$SUSPENSION/resume" '{}'
check "resume answered 201" "$STATUS" 201
[[ "$STATUS" == 201 ]] || die "resume answered $STATUS: $(cat "$LAST")"
RESUMED="$(field 'd["result"]["instance_id"]')"
INSTANCE="$RESUMED"
record resume_s "$SECONDS_TAKEN"
check "the resumed sandbox is a new Instance" "$([[ "$RESUMED" != "$SUSPENDED" ]] && echo new || echo same)" new

guest after sh -c 'cat /run/proof/tmpfs-value; cat /run/proof/counter; cat /run/proof/counter.pid
kill -0 "$(cat /run/proof/counter.pid)" && echo alive; sleep 1; cat /run/proof/counter; cat /root/disk-value'
check "the command ran" "$GUEST_EXIT" 0
check "the tmpfs value survived" "$(sed -n 1p "$GUEST_OUT")" "$TMPFS_VALUE"
check "the same counter process" "$(sed -n 3p "$GUEST_OUT")" "$COUNTER_PID"
check "the counter process is alive" "$(sed -n 4p "$GUEST_OUT")" alive
AFTER_1="$(sed -n 2p "$GUEST_OUT")"
AFTER_2="$(sed -n 5p "$GUEST_OUT")"
check "the counter continued from where it was" "$(( AFTER_1 >= COUNT_BEFORE ))" 1
check "the counter keeps counting" "$(( AFTER_2 > AFTER_1 ))" 1
check "the disk value survived" "$(sed -n 6p "$GUEST_OUT")" "$TMPFS_VALUE"

log "the Suspension resumes once"
api resume-again POST "/v1/suspensions/$SUSPENSION/resume" '{}'
check "a second resume is refused as unknown" "$STATUS" 404

log "destroying"
api destroy DELETE "/v1/sandboxes/$RESUMED"
check "destroy answered 200" "$STATUS" 200
RESUMED=""
INSTANCE=""
printf '\n%s checks failed\n' "$failures" | tee -a "$OUT/checks.txt"
(( failures == 0 ))
