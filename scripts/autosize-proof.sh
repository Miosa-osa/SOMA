#!/usr/bin/env bash
#
# The automatic sizing proof with one command: a sandbox kept between a minimum and a maximum of
# processors and memory shrinks while idle, grows under a CPU load and under a memory load, and
# shrinks back once the load ends, live, with every change in its scaling history.
#
# It starts a private soma-api on its own loopback port and state root, so a soma-api already
# serving this host is not touched. The Generation must have been prepared with room to grow:
# several vCPUs (SOMA_PREPARE_VCPUS) and a memory ceiling (prepare-generation.sh's ceiling_mib).
# Every request and response is kept under <work>/results.
#
# Usage:
#   scripts/autosize-proof.sh --store DIR --reference REF --work DIR \
#       --vcpus N --memory-mib M --storage-mib S [--port PORT]
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
STORE="" REFERENCE="" WORK="" PORT=18932 VCPUS="" MEMORY="" STORAGE=""
while (( $# > 0 )); do
    case "$1" in
        --store) STORE="$2"; shift 2 ;;
        --reference) REFERENCE="$2"; shift 2 ;;
        --work) WORK="$2"; shift 2 ;;
        --port) PORT="$2"; shift 2 ;;
        --vcpus) VCPUS="$2"; shift 2 ;;
        --memory-mib) MEMORY="$2"; shift 2 ;;
        --storage-mib) STORAGE="$2"; shift 2 ;;
        *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
die() { printf 'autosize-proof: %s\n' "$1" >&2; exit 1; }
[[ -n "$STORE" && -n "$REFERENCE" && -n "$WORK" && -n "$VCPUS" && -n "$MEMORY" && -n "$STORAGE" ]] \
    || die "--store, --reference, --work, --vcpus, --memory-mib and --storage-mib are required"
[[ -x "$REPO_ROOT/target/release/soma-api" ]] || die "target/release/soma-api is missing"
mkdir -p "$WORK"
WORK="$(cd "$WORK" && pwd -P)"
[[ ! -e "$WORK/results" ]] || die "$WORK/results already exists; use a new --work directory"
OUT="$WORK/results"
mkdir -p "$OUT" "$WORK/state" "$WORK/heads"
export SOMA_PROOF_API="127.0.0.1:$PORT" SOMA_PROOF_TENANT=autosize-proof
# shellcheck source=scripts/desktop-proof/api.sh
source "$REPO_ROOT/scripts/desktop-proof/api.sh"
log() { printf '\n==> %s\n' "$1" | tee -a "$OUT/calls.log"; }
failures=0
check() {
    if [[ "$2" == "$3" ]]; then
        printf 'ok    %s\n' "$1" | tee -a "$OUT/checks.txt"
    else
        printf 'FAIL  %s: expected %s, saw %s\n' "$1" "$3" "$2" | tee -a "$OUT/checks.txt"
        failures=$((failures + 1))
    fi
}
# sizing: the sandbox's processor limit and memory, read from the autosize route.
sizing() {
    api autosize GET "/v1/sandboxes/$INSTANCE/autosize"
    CPU="$(field 'd["result"]["usage"]["cpu_limit"] if d["result"]["usage"] else 0')"
    MEM="$(field 'd["result"]["usage"]["memory_total_mb"] if d["result"]["usage"] else 0')"
    printf '%s\tcpu=%s\tmemory_mb=%s\n' "$(date -u +%T)" "$CPU" "$MEM" | tee -a "$OUT/sizes.tsv"
}
# watch <seconds>: reads the sizing every five seconds.
watch() { for _ in $(seq 1 $(( $1 / 5 ))); do sleep 5; sizing; done; }

# CPU limits are written to a delegated cgroup2 subtree; one is made for the proof when the host
# names none (a production host sets SOMA_JAIL_CGROUP_ROOT for soma-api the same way).
if [[ -z "${SOMA_JAIL_CGROUP_ROOT:-}" ]]; then
    SOMA_JAIL_CGROUP_ROOT=/sys/fs/cgroup/soma-autosize-proof
    grep -qw cpu /sys/fs/cgroup/cgroup.subtree_control || echo +cpu >/sys/fs/cgroup/cgroup.subtree_control
    mkdir -p "$SOMA_JAIL_CGROUP_ROOT"
    echo +cpu >"$SOMA_JAIL_CGROUP_ROOT/cgroup.subtree_control" 2>/dev/null || true
fi
export SOMA_JAIL_CGROUP_ROOT

log "starting a private soma-api on $SOMA_PROOF_API"
SOMA_GENERATION_STORE="$STORE" SOMA_HEAD_DIR="$WORK/heads" \
    SOMA_MACHINE_HOST_LOG="$OUT/machine-host.log" \
    "$REPO_ROOT/target/release/soma-api" --listen "$SOMA_PROOF_API" --backend kvm \
    --state-root "$WORK/state" --workers 8 >"$OUT/soma-api.log" 2>&1 &
API_PID=$!
INSTANCE=""
cleanup() {
    [[ -n "$INSTANCE" ]] && curl -sS -m 120 -X DELETE "http://$SOMA_PROOF_API/v1/sandboxes/$INSTANCE" \
        -H "x-soma-tenant: $SOMA_PROOF_TENANT" >/dev/null 2>&1
    kill "$API_PID" 2>/dev/null || true
    wait "$API_PID" 2>/dev/null || true
}
trap cleanup EXIT
for _ in $(seq 1 100); do
    curl -sS -m 2 -o /dev/null "http://$SOMA_PROOF_API/v1/version" \
        -H "x-soma-tenant: $SOMA_PROOF_TENANT" 2>/dev/null && break
    sleep 0.1
done

log "creating one sandbox at $VCPUS vCPUs and $MEMORY MiB"
SHAPE="{\"vcpu_count\":$VCPUS,\"memory_mib\":$MEMORY,\"storage_mib\":$STORAGE,\"capabilities\":{\"network\":$RUNTIME_DEFAULT_NETWORK}}"
api create POST /v1/sandboxes "$(printf '{"image":"%s","shape":%s}' "$REFERENCE" "$SHAPE")"
[[ "$STATUS" == 201 ]] || die "create answered $STATUS: $(cat "$LAST")"
INSTANCE="$(field 'd["result"]["instance_id"]')"

log "turning automatic sizing on: 1 to $VCPUS processors, $MEMORY MiB up to the ceiling"
api autosize-on PUT "/v1/sandboxes/$INSTANCE/autosize" "{\"min_cpu\":1,\"max_cpu\":$VCPUS}"
check "autosize answered 200" "$STATUS" 200
check "autosize is on" "$(field 'd["result"]["enabled"]')" True

log "idle: the processors shrink"
watch 60
check "an idle sandbox shrank its processors" "$(( CPU < VCPUS ))" 1
IDLE_CPU=$CPU

log "a CPU load on every processor: the processors grow"
guest cpuload /bin/sh -c "for i in \$(seq 1 $VCPUS); do timeout 40 sh -c 'while :; do :; done' & done; wait" &
LOAD=$!
watch 35
wait "$LOAD" || true
check "a busy sandbox grew its processors" "$(( CPU > IDLE_CPU ))" 1

log "a memory load: the memory grows"
# Grown a block every two seconds, so the controller sees the guest run short and adds memory
# before the next block arrives, up to half again the boot size.
CHUNKS=$(( MEMORY * 3 / 2 / 256 ))
guest memload /bin/sh -c "timeout 100 python3 -c 'import time
a = []
for _ in range($CHUNKS):
    a.append(b\"\\x01\" * (256 << 20)); time.sleep(2)
time.sleep(30)'" &
LOAD=$!
watch 60
GROWN=$MEM
wait "$LOAD" || true
check "a sandbox short of memory grew it" "$(( GROWN > MEMORY ))" 1

log "the load ends: the sandbox shrinks back"
# A shrink asked while the load still held its memory is refused and asked again after two
# minutes, so this waits three.
watch 180
check "memory came back down" "$(( MEM < GROWN ))" 1

api history GET "/v1/sandboxes/$INSTANCE/autosize"
field 'json.dumps(d["result"]["history"], indent=1)' >"$OUT/history.json" 2>/dev/null \
    || python3 -c 'import json,sys; print(json.dumps(json.load(open(sys.argv[1]))["result"]["history"], indent=1))' "$LAST" >"$OUT/history.json"
check "the history recorded the changes" "$(( $(grep -c '"dimension"' "$OUT/history.json") >= 3 ))" 1
printf '\n%s checks failed\n' "$failures" | tee -a "$OUT/checks.txt"
(( failures == 0 ))
