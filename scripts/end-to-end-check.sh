#!/usr/bin/env bash
#
# Walks the whole documented flow on one host and says which stage broke.
#
# Unit tests prove parts. This proves the join: host readiness, build, Generation compilation,
# snapshot capture, a sandbox that runs a command through the public command line, and a cleanup
# that left nothing behind. Every stage records passed, failed, or skipped with a bounded detail
# string, results are written machine readable, and the summary names the first failing stage. A
# failure is never worked around: later stages are marked skipped so the break stays visible.
# It never writes to a shared store, compiling its own Generation into its own work directory and
# running against its own state root and head directory, so the cleanup proof can be exact.
#
# Usage:
#   end-to-end-check.sh [--image REF] [--work DIR] [--fs-tools DIR] [--expect REGEX]
#                       [--with-host-setup] [--purge] [--command CMD ARG...]

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
readonly MUSL_TARGET="x86_64-unknown-linux-musl"
cd "$REPO_ROOT" || exit 2

IMAGE="node:22"
WORK=""
FS_TOOLS="$REPO_ROOT/fs-tools"
EXPECT="^v[0-9]"
GUEST_COMMAND=(/usr/local/bin/node --version)
RUN_SETUP=0
PURGE=0
DETAIL=""
FIRST_FAILURE=""
ENTRY=""
STATE_BASE=""
declare -a NAMES=() STATUSES=() DETAILS=() DURATIONS=()

log() { printf '\n==> %s\n' "$1"; }
die() { printf '%s\n' "$1" >&2; exit 2; }

parse_args() {
    while (( $# > 0 )); do
        case "$1" in
            --image) IMAGE="$2"; shift 2 ;;
            --work) WORK="$2"; shift 2 ;;
            --fs-tools) FS_TOOLS="$2"; shift 2 ;;
            --expect) EXPECT="$2"; shift 2 ;;
            --with-host-setup) RUN_SETUP=1; shift ;;
            --purge) PURGE=1; shift ;;
            --command) shift; GUEST_COMMAND=("$@"); break ;;
            *) die "unknown argument: $1" ;;
        esac
    done
    [[ -n "$WORK" ]] || WORK="$(mktemp -d "${TMPDIR:-/tmp}/soma-e2e.XXXXXXXX")"
    mkdir -p "$WORK" || die "cannot create the work directory $WORK"
    WORK="$(cd "$WORK" && pwd -P)"
    STORE="$WORK/prepared"
    HEAD_DIR="$WORK/heads"
    STATE_ROOT="$WORK/state"
    LOG_DIR="$WORK/logs"
    mkdir -p "$STORE" "$HEAD_DIR" "$STATE_ROOT" "$LOG_DIR"
    if ! command -v cargo >/dev/null 2>&1 && [[ -r "${CARGO_HOME:-$HOME/.cargo}/env" ]]; then
        # setup-host.sh installs rustup with --no-modify-path, so a non-login shell lacks it.
        # shellcheck disable=SC1091
        source "${CARGO_HOME:-$HOME/.cargo}/env"
    fi
}

bound() {
    tr -d '\000-\010\013\014\016-\037' | tr '\n\t' '  ' \
        | sed 's/\\/\\\\/g; s/"/\\"/g' | cut -c1-200
}

record() {
    NAMES+=("$1"); STATUSES+=("$2"); DETAILS+=("$3"); DURATIONS+=("$4")
    printf '  %-8s %-18s %s\n' "$2" "$1" "$3"
}

run_stage() {
    local name="$1" fn="$2" started ended status
    DETAIL=""
    if [[ -n "$FIRST_FAILURE" ]]; then
        record "$name" skipped "not attempted after $FIRST_FAILURE failed" 0
        return
    fi
    log "$name"
    started="$(date +%s%3N)"
    if "$fn" >"$LOG_DIR/$name.log" 2>&1; then status=passed; else status=failed; fi
    ended="$(date +%s%3N)"
    [[ -n "$DETAIL" ]] || DETAIL="$(tail -n 1 "$LOG_DIR/$name.log" 2>/dev/null)"
    [[ "$status" == passed ]] || FIRST_FAILURE="$name"
    record "$name" "$status" "$(printf '%s' "$DETAIL" | bound)" "$(( ended - started ))"
}

# Stage 1. The host carries what setup-host.sh and build-fs-tools.sh are supposed to leave behind.
stage_host_setup() {
    local notes=() failed=0
    if (( RUN_SETUP )); then
        ./scripts/setup-host.sh || return 1
        ./scripts/build-fs-tools.sh "$FS_TOOLS" || return 1
    fi
    check() {
        local label="$1"; shift
        if "$@" >/dev/null 2>&1; then notes+=("$label=ok"); else notes+=("$label=MISSING"); failed=1; fi
    }
    check kvm bash -c '[ -c /dev/kvm ] && [ -w /dev/kvm ]'
    check virt grep -Eq 'vmx|svm' /proc/cpuinfo
    check cgroup2 test -f /sys/fs/cgroup/cgroup.controllers
    check skopeo command -v skopeo
    check cargo command -v cargo
    check python3 command -v python3
    check docker docker info
    check musl bash -c "rustup target list --installed | grep -qx '$MUSL_TARGET'"
    check fs_tools bash -c "ls '$FS_TOOLS/erofs' '$FS_TOOLS/e2fsprogs' | grep -q ."
    notes+=("head_fs=$(findmnt -no FSTYPE --target "$HEAD_DIR" 2>/dev/null || echo unknown)")
    DETAIL="${notes[*]}"
    return "$failed"
}

# Stage 2. The three artifacts a KVM host launches from, built from this checkout.
stage_build() {
    ./scripts/build-soma.sh || return 1
    local kernel agent="target/$MUSL_TARGET/release/soma-guest-agent"
    kernel="$(compgen -G 'kernel/out/vmlinux-*-soma-v1' | head -1)"
    [[ -x target/release/soma ]] || { DETAIL="target/release/soma is missing"; return 1; }
    [[ -f "$agent" ]] || { DETAIL="the guest agent is missing"; return 1; }
    [[ -n "$kernel" ]] || { DETAIL="no built kernel under kernel/out"; return 1; }
    DETAIL="soma=$(sha256sum target/release/soma | cut -c1-12) kernel=$(basename "$kernel")"
    DETAIL="$DETAIL agent=$(sha256sum "$agent" | cut -c1-12)"
}

# Stage 3. One OCI image compiled into a launchable Generation in a private prepared store.
stage_prepare_generation() {
    ./scripts/prepare-generation.sh "$IMAGE" "$STORE" "$FS_TOOLS" || return 1
    ENTRY="$(compgen -G "$STORE/ref-*" | head -1)"
    [[ -n "$ENTRY" && -d "$ENTRY" ]] || { DETAIL="no prepared entry was published"; return 1; }
    [[ -f "$ENTRY/candidate.somacan" ]] || { DETAIL="the entry has no candidate"; return 1; }
    [[ -d "$ENTRY/store" ]] || { DETAIL="the entry has no artifact store"; return 1; }
    local reference
    reference="$(cat "$ENTRY/reference")"
    [[ "$reference" == "$IMAGE" ]] || { DETAIL="entry names $reference, not $IMAGE"; return 1; }
    DETAIL="entry=$(basename "$ENTRY") candidate=$(stat -c%s "$ENTRY/candidate.somacan")B"
    DETAIL="$DETAIL store=$(du -sh "$ENTRY/store" | cut -f1)"
}

# Stage 4. The snapshot every later launch restores from, taken at the agent's repair point.
stage_capture_snapshot() {
    cargo build --release -p soma-local --example capture_snapshot || return 1
    ./target/release/examples/capture_snapshot "$ENTRY" || return 1
    local state="$ENTRY/snapshot/state.somasnap" overlay="$ENTRY/snapshot/overlay.raw"
    [[ -f "$state" ]] || { DETAIL="no snapshot state was written"; return 1; }
    [[ -f "$overlay" ]] || { DETAIL="no sterile overlay template was written"; return 1; }
    DETAIL="state=$(du -h "$state" | cut -f1) overlay=$(du -h "$overlay" | cut -f1)"
}

soma_run() {
    local tag="$1"
    SOMA_GENERATION_STORE="$STORE" \
    SOMA_HEAD_DIR="$HEAD_DIR" \
    SOMA_ALLOW_UNCERTIFIED_GENERATION=1 \
        ./target/release/soma --format json --backend kvm --state-root "$STATE_ROOT" \
        run "$IMAGE" -- "${GUEST_COMMAND[@]}" \
        >"$WORK/run-$tag.json" 2>"$WORK/run-$tag.err"
    python3 ./scripts/end-to-end/inspect-run.py "$WORK/run-$tag.json" "$EXPECT"
}

# Stage 5. Two sandboxes, one after the other, each running the command and reporting its cleanup.
# Two rather than one because the first creates the durable state a fresh state root has never
# held, and only the second can show that a steady-state run adds nothing to it.
stage_run_sandbox() {
    probe before || return 1
    local first second
    first="$(soma_run first)" || { DETAIL="first run: $first"; return 1; }
    STATE_BASE="$(state_fingerprint)"
    second="$(soma_run second)" || { DETAIL="second run: $second"; return 1; }
    DETAIL="first: $first | second: $second"
}

# The three shared host surfaces a leak would show up on, sampled either side of the runs. An
# empty table listing is ambiguous, because a host that holds many refuses to show them without
# privilege, so the listing command's own status decides and an unreadable one never reads as empty.
nft_tables() { nft list tables 2>/dev/null || sudo -n nft list tables 2>/dev/null; }

# Bytes alone would miss an empty file or an empty directory, so the entry count is held with them.
state_fingerprint() {
    printf '%s bytes in %s entries' \
        "$(du -sb "$STATE_ROOT" | cut -f1)" "$(find "$STATE_ROOT" | wc -l)"
}

probe() {
    local when="$1"
    pgrep -a -f "$STATE_ROOT" 2>/dev/null | sort >"$WORK/procs.$when"
    ip netns list 2>/dev/null | awk '{ print $1 }' | sort >"$WORK/netns.$when"
    if ! { nft_tables | sort >"$WORK/nft.$when"; }; then
        printf 'unreadable\n' >"$WORK/nft.$when"
    fi
    return 0
}

# Stage 6. Nothing of the sandbox outlived it.
stage_cleanup_proof() {
    probe after
    local checks="$WORK/cleanup-checks.tsv" leaked=0 notes=()
    : >"$checks"
    verdict() {
        printf '%s\t%s\t%s\n' "$1" "$2" "$3" >>"$checks"
        notes+=("$1=$2")
        [[ "$2" != leaked ]] || leaked=1
    }
    local heads after
    # The head root holds one directory per shard, so a leak is a file left inside a shard
    # rather than any entry under the root.
    heads="$(find "$HEAD_DIR" -mindepth 1 -type f | wc -l)"
    if (( heads == 0 )); then verdict overlay_heads clean "no head file is left under the head root"
    else verdict overlay_heads leaked "$heads head files left under $HEAD_DIR"; fi
    differed processes "$WORK/procs.before" "$WORK/procs.after" "processes naming the state root"
    differed netns "$WORK/netns.before" "$WORK/netns.after" "network namespaces"
    if grep -qx unreadable "$WORK/nft.after"; then
        verdict nftables unverified "the table listing could not be read, so nothing was compared"
    else
        differed nftables "$WORK/nft.before" "$WORK/nft.after" "nftables tables"
    fi
    after="$(state_fingerprint)"
    if [[ "$after" == "$STATE_BASE" ]]; then
        verdict state_root clean "$after, unchanged across the second run"
    else
        verdict state_root leaked "went from $STATE_BASE to $after across one run"
    fi
    DETAIL="${notes[*]}"
    return "$leaked"
}

differed() {
    local name="$1" what="$4" added
    added="$(comm -13 "$2" "$3" | bound)"
    if [[ -z "$added" ]]; then verdict "$name" clean "no $what appeared"
    else verdict "$name" leaked "$what appeared: $added"; fi
}

write_results() {
    local results="$WORK/results.json" index=0 separator="" failure=null
    [[ -z "$FIRST_FAILURE" ]] || failure="\"$FIRST_FAILURE\""
    {
        printf '{"schema":"soma.e2e.v1","image":"%s","host":"%s","commit":"%s",' \
            "$IMAGE" "$(uname -srm | bound)" \
            "${SOMA_E2E_COMMIT:-$(git rev-parse HEAD 2>/dev/null || echo unknown)}"
        printf '"work":"%s","first_failure":%s,"stages":[' "$WORK" "$failure"
        while (( index < ${#NAMES[@]} )); do
            printf '%s{"stage":"%s","status":"%s","detail":"%s","duration_ms":%s,"log":"%s"}' \
                "$separator" "${NAMES[index]}" "${STATUSES[index]}" "${DETAILS[index]}" \
                "${DURATIONS[index]}" "logs/${NAMES[index]}.log"
            separator=","
            index=$(( index + 1 ))
        done
        printf ']}\n'
    } >"$results"
    printf '%s\n' "$results"
}

summarize() {
    log "summary"
    # `read` assigns into the caller's scope unless the name is local, so every name it fills is
    # declared. An undeclared `status` here once silently overwrote main's own exit status.
    local index=0 check outcome detail
    while (( index < ${#NAMES[@]} )); do
        printf '  %-8s %-18s %6s ms  %s\n' "${STATUSES[index]}" "${NAMES[index]}" \
            "${DURATIONS[index]}" "${DETAILS[index]}"
        index=$(( index + 1 ))
    done
    if [[ -f "$WORK/cleanup-checks.tsv" ]]; then
        printf '\n  cleanup checks:\n'
        while IFS=$'\t' read -r check outcome detail; do
            printf '    %-14s %-11s %s\n' "$check" "$outcome" "$detail"
        done <"$WORK/cleanup-checks.tsv"
    fi
    printf '\n  results: %s\n  logs:    %s\n' "$(write_results)" "$LOG_DIR"
    if [[ -n "$FIRST_FAILURE" ]]; then
        printf '\nFAILED at stage %s. Read %s.\n' "$FIRST_FAILURE" "$LOG_DIR/$FIRST_FAILURE.log"
        return 1
    fi
    printf '\nEvery stage passed and the sandbox left nothing behind.\n'
}

main() {
    parse_args "$@"
    [[ "$(uname -s)" == Linux ]] || die "the end to end check needs a Linux KVM host"
    log "work directory $WORK"
    run_stage host_setup stage_host_setup
    run_stage build stage_build
    run_stage prepare_generation stage_prepare_generation
    run_stage capture_snapshot stage_capture_snapshot
    run_stage run_sandbox stage_run_sandbox
    run_stage cleanup_proof stage_cleanup_proof
    local status=0
    summarize || status=1
    # The prepared store, heads, and state root are large; the results and logs are the record.
    (( ! PURGE )) || rm -rf -- "$STORE" "$HEAD_DIR" "$STATE_ROOT"
    return "$status"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
