#!/usr/bin/env bash
#
# Proves live memory resize on a real KVM host, end to end, through the public command line.
#
# The device tests prove the protocol against a model driver. This proves the join with a real
# guest kernel: memory plugged into a running machine shows up in the guest's own MemTotal, the
# guest can use it, and giving it back returns it to the host. One machine is launched and
# taken through plug, use, unplug and the edges, and every stage records passed, failed, or
# skipped. A failure is never worked around; later stages are marked skipped.
#
# The stages:
#   host_setup, build, prepare_generation, capture_snapshot   as end-to-end-check.sh
#   launch        a machine with BOOT_MIB of memory, its guest MemTotal and host RSS recorded
#   plug          resize to BOOT_MIB + PLUG_MIB; the guest's MemTotal grows by exactly that
#   touch         the guest fills TOUCH_MIB of it; the host VMM's resident memory follows
#   unplug        resize back to BOOT_MIB; MemTotal returns and the host gets the memory back
#   bounds        past the ceiling and below the boot size change nothing they should not
#   cleanup       the machine is destroyed and no process naming it is left
#
# Resizing uses the command line verb `soma machine resize --instance-id ID --memory-mib N`,
# which prints the usual envelope with `result.memory`. If the verb is named differently, pass
# --resize-cmd with {id} and {mib} placeholders. It never writes to a shared store.
#
# Usage:
#   live-memory-resize.sh [--image REF] [--work DIR] [--fs-tools DIR] [--boot-mib N]
#                         [--plug-mib N] [--touch-mib N] [--min-return-mib N]
#                         [--resize-cmd TEMPLATE] [--with-host-setup] [--purge]

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly JUDGE="$REPO_ROOT/scripts/live-memory-resize/judge.py"
cd "$REPO_ROOT" || exit 2

# shellcheck source=scripts/end-to-end-record.sh
source "$REPO_ROOT/scripts/end-to-end-record.sh"
# shellcheck source=scripts/live-memory-resize/host.sh
source "$REPO_ROOT/scripts/live-memory-resize/host.sh"

IMAGE="node:22"
WORK=""
FS_TOOLS="$REPO_ROOT/fs-tools"
BOOT_MIB=1024
PLUG_MIB=2048
TOUCH_MIB=1536
MIN_RETURN_MIB=1024
RESIZE_CMD=""
RUN_SETUP=0
PURGE=0
DETAIL=""
FIRST_FAILURE=""
ENTRY=""
INSTANCE=""
HOST_PID=""
MEMTOTAL0=0
RSS0=0
RSS_PEAK=0
declare -a NAMES=() STATUSES=() DETAILS=() DURATIONS=()

log() { printf '\n==> %s\n' "$1"; }
die() { printf '%s\n' "$1" >&2; exit 2; }

parse_args() {
    while (( $# > 0 )); do
        case "$1" in
            --image) IMAGE="$2"; shift 2 ;;
            --work) WORK="$2"; shift 2 ;;
            --fs-tools) FS_TOOLS="$2"; shift 2 ;;
            --boot-mib) BOOT_MIB="$2"; shift 2 ;;
            --plug-mib) PLUG_MIB="$2"; shift 2 ;;
            --touch-mib) TOUCH_MIB="$2"; shift 2 ;;
            --min-return-mib) MIN_RETURN_MIB="$2"; shift 2 ;;
            --resize-cmd) RESIZE_CMD="$2"; shift 2 ;;
            --with-host-setup) RUN_SETUP=1; shift ;;
            --purge) PURGE=1; shift ;;
            *) die "unknown argument: $1" ;;
        esac
    done
    (( PLUG_MIB % 2 == 0 && PLUG_MIB > 0 )) || die "--plug-mib must be a positive multiple of 2"
    (( TOUCH_MIB < PLUG_MIB )) || die "--touch-mib must be less than --plug-mib"
    [[ -n "$WORK" ]] || WORK="$(mktemp -d "${TMPDIR:-/tmp}/soma-memresize.XXXXXXXX")"
    mkdir -p "$WORK" || die "cannot create the work directory $WORK"
    WORK="$(cd "$WORK" && pwd -P)"
    STORE="$WORK/prepared"; HEAD_DIR="$WORK/heads"; STATE_ROOT="$WORK/state"; LOG_DIR="$WORK/logs"
    mkdir -p "$STORE" "$HEAD_DIR" "$STATE_ROOT" "$LOG_DIR"
    if ! command -v cargo >/dev/null 2>&1 && [[ -r "${CARGO_HOME:-$HOME/.cargo}/env" ]]; then
        # shellcheck disable=SC1091
        source "${CARGO_HOME:-$HOME/.cargo}/env"
    fi
}

soma() {
    SOMA_GENERATION_STORE="$STORE" SOMA_HEAD_DIR="$HEAD_DIR" SOMA_ALLOW_UNCERTIFIED_GENERATION=1 \
        ./target/release/soma --format json --backend kvm --state-root "$STATE_ROOT" "$@"
}

stage_host_setup() {
    if (( RUN_SETUP )); then
        ./scripts/setup-host.sh || return 1
        ./scripts/build-fs-tools.sh "$FS_TOOLS" || return 1
    fi
    local missing=()
    [[ -c /dev/kvm && -w /dev/kvm ]] || missing+=("writable /dev/kvm")
    [[ "$(uname -m)" == x86_64 ]] || missing+=("x86_64")
    [[ -r /proc/self/smaps_rollup ]] || missing+=("smaps_rollup")
    for tool in cargo python3 skopeo; do
        command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
    done
    ls "$FS_TOOLS/erofs" "$FS_TOOLS/e2fsprogs" >/dev/null 2>&1 || missing+=("fs-tools in $FS_TOOLS")
    DETAIL="kvm host, boot=${BOOT_MIB}MiB plug=${PLUG_MIB}MiB touch=${TOUCH_MIB}MiB"
    (( ${#missing[@]} == 0 )) || { DETAIL="missing: ${missing[*]}"; return 1; }
}

stage_build() {
    ./scripts/build-soma.sh || return 1
    [[ -x target/release/soma ]] || { DETAIL="target/release/soma is missing"; return 1; }
    DETAIL="soma=$(sha256sum target/release/soma | cut -c1-12)"
}

stage_prepare_generation() {
    ./scripts/prepare-generation.sh "$IMAGE" "$STORE" "$FS_TOOLS" "$BOOT_MIB" || return 1
    ENTRY="$(compgen -G "$STORE/ref-*" | head -1)"
    [[ -n "$ENTRY" && -f "$ENTRY/candidate.somacan" ]] || { DETAIL="no prepared entry"; return 1; }
    DETAIL="entry=$(basename "$ENTRY")"
}

stage_capture_snapshot() {
    cargo build --release -p soma-local --example capture_snapshot || return 1
    ./target/release/examples/capture_snapshot "$ENTRY" "$BOOT_MIB" || return 1
    [[ -f "$ENTRY/snapshot/state.somasnap" ]] || { DETAIL="no snapshot state was written"; return 1; }
    DETAIL="snapshot at ${BOOT_MIB}MiB"
}

stage_launch() {
    soma machine launch --memory-mib "$BOOT_MIB" "$IMAGE" >"$WORK/launch.json" 2>"$WORK/launch.err" \
        || { DETAIL="launch: $(tail -c 160 "$WORK/launch.err")"; return 1; }
    INSTANCE="$(python3 "$JUDGE" instance "$WORK/launch.json")" || { DETAIL="$INSTANCE"; return 1; }
    HOST_PID="$(host_pid "$INSTANCE" "$STATE_ROOT")"
    [[ -n "$HOST_PID" ]] || { DETAIL="no process hosts machine $INSTANCE"; return 1; }
    MEMTOTAL0="$(guest_memtotal_kib)" || { DETAIL="MemTotal unreadable: $MEMTOTAL0"; return 1; }
    RSS0="$(host_rss_kib "$HOST_PID")"
    DETAIL="instance=${INSTANCE:0:8} pid=$HOST_PID MemTotal=$((MEMTOTAL0 / 1024))MiB host_rss=$((RSS0 / 1024))MiB"
    # The guest keeps some of its boot memory for the kernel, so it sees a little less.
    (( MEMTOTAL0 > BOOT_MIB * 1024 * 85 / 100 && MEMTOTAL0 <= BOOT_MIB * 1024 )) \
        || { DETAIL="$DETAIL, not near the ${BOOT_MIB}MiB it booted with"; return 1; }
}

stage_plug() {
    local outcome state plugged total booted ceiling grown
    outcome="$(resize_to $(( BOOT_MIB + PLUG_MIB )))" || { DETAIL="$outcome"; return 1; }
    read -r state plugged total booted ceiling <<<"$outcome"
    [[ "$state" == applied_live ]] || { DETAIL="state=$state, not applied_live"; return 1; }
    (( plugged == PLUG_MIB && total == booted + plugged )) \
        || { DETAIL="the device confirms plugged=$plugged total=$total, asked $PLUG_MIB"; return 1; }
    wait_memtotal_at_least $(( MEMTOTAL0 + PLUG_MIB * 1024 * 98 / 100 )) 30 \
        || { DETAIL="the guest MemTotal did not grow by ${PLUG_MIB}MiB: $(guest_memtotal_kib)KiB"; return 1; }
    grown=$(( $(guest_memtotal_kib) - MEMTOTAL0 ))
    (( grown <= PLUG_MIB * 1024 )) \
        || { DETAIL="the guest gained ${grown}KiB, more than the ${PLUG_MIB}MiB plugged"; return 1; }
    DETAIL="plugged=${plugged}MiB ceiling=${ceiling}MiB guest gained $((grown / 1024))MiB"
}

stage_touch() {
    local peak
    # Anonymous memory the guest fills and holds while the host side is sampled.
    ( guest_exec /usr/local/bin/node -e \
        "const b=Buffer.alloc($TOUCH_MIB*1048576,1);setTimeout(()=>console.log(b.length),6000)" \
        >"$WORK/touch.out" 2>&1 ) &
    local worker=$!
    peak="$(sample_peak_rss_kib "$HOST_PID" "$worker")"
    wait "$worker" || { DETAIL="the guest allocation failed: $(tail -c 160 "$WORK/touch.out")"; return 1; }
    RSS_PEAK="$peak"
    DETAIL="host rss $((RSS0 / 1024))MiB -> peak $((peak / 1024))MiB while the guest held ${TOUCH_MIB}MiB"
    (( peak - RSS0 >= TOUCH_MIB * 1024 * 75 / 100 )) \
        || { DETAIL="$DETAIL, the host did not follow the guest"; return 1; }
}

stage_unplug() {
    local outcome state plugged total after
    outcome="$(resize_to "$BOOT_MIB")" || { DETAIL="$outcome"; return 1; }
    read -r state plugged total _ _ <<<"$outcome"
    [[ "$state" == applied_live ]] || { DETAIL="state=$state, not applied_live: $outcome"; return 1; }
    (( plugged == 0 )) || { DETAIL="the device still holds ${plugged}MiB"; return 1; }
    wait_memtotal_at_most $(( MEMTOTAL0 + MEMTOTAL0 / 100 )) 60 \
        || { DETAIL="the guest MemTotal did not return: $(guest_memtotal_kib)KiB vs $MEMTOTAL0"; return 1; }
    after="$(wait_rss_below "$HOST_PID" $(( RSS_PEAK - MIN_RETURN_MIB * 1024 )) 30)"
    DETAIL="MemTotal back to $(( $(guest_memtotal_kib) / 1024 ))MiB, host rss peak $((RSS_PEAK / 1024)) -> $((after / 1024))MiB"
    (( after <= RSS_PEAK - MIN_RETURN_MIB * 1024 )) \
        || { DETAIL="$DETAIL, less than ${MIN_RETURN_MIB}MiB came back to the host"; return 1; }
}

stage_bounds() {
    local outcome state plugged total booted ceiling
    # A request past the ceiling is either refused outright or capped; it never exceeds it.
    outcome="$(resize_to 1048576)" || outcome=refused
    if [[ "$outcome" != refused ]]; then
        read -r state plugged total booted ceiling <<<"$outcome"
        (( total <= ceiling && plugged <= ceiling - booted )) \
            || { DETAIL="a huge request gave total=$total over ceiling=$ceiling"; return 1; }
    fi
    wait_memtotal_at_most $(( MEMTOTAL0 + (${ceiling:-$BOOT_MIB} - ${booted:-$BOOT_MIB}) * 1024 + MEMTOTAL0 / 100 )) 60 \
        || { DETAIL="the guest holds more than the ceiling allows: $(guest_memtotal_kib)KiB"; return 1; }
    resize_to "$BOOT_MIB" >/dev/null || true
    wait_memtotal_at_most $(( MEMTOTAL0 + MEMTOTAL0 / 100 )) 60 \
        || { DETAIL="MemTotal did not come back to the boot size"; return 1; }
    # Below the boot size there is nothing to take: refused or unchanged, never a shrink.
    resize_to $(( BOOT_MIB / 2 )) >/dev/null || true
    sleep 2
    (( $(guest_memtotal_kib) >= MEMTOTAL0 - MEMTOTAL0 / 100 )) \
        || { DETAIL="a request below the boot size shrank the guest to $(guest_memtotal_kib)KiB"; return 1; }
    DETAIL="past the ceiling: ${outcome%% *}; below the boot size: the guest kept its boot memory"
}

stage_cleanup() {
    soma machine destroy --instance-id "$INSTANCE" >"$WORK/destroy.json" 2>"$WORK/destroy.err" \
        || { DETAIL="destroy: $(tail -c 160 "$WORK/destroy.err")"; return 1; }
    local waited=0
    while kill -0 "$HOST_PID" 2>/dev/null && (( waited < 30 )); do sleep 1; waited=$(( waited + 1 )); done
    ! kill -0 "$HOST_PID" 2>/dev/null || { DETAIL="process $HOST_PID outlived destroy"; return 1; }
    DETAIL="destroyed; the hosting process exited"
}

main() {
    parse_args "$@"
    [[ "$(uname -s)" == Linux ]] || die "the live memory resize check needs a Linux KVM host"
    log "work directory $WORK"
    run_stage host_setup stage_host_setup
    run_stage build stage_build
    run_stage prepare_generation stage_prepare_generation
    run_stage capture_snapshot stage_capture_snapshot
    run_stage launch stage_launch
    run_stage plug stage_plug
    run_stage touch stage_touch
    run_stage unplug stage_unplug
    run_stage bounds stage_bounds
    run_stage cleanup stage_cleanup
    local status=0
    summarize || status=1
    (( ! PURGE )) || rm -rf -- "$STORE" "$HEAD_DIR" "$STATE_ROOT"
    return "$status"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
