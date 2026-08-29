#!/usr/bin/env bash
# Container side of the XFS reflink measurement.
#
# Runs as root inside the privileged image built from
# scripts/xfs-reflink-bench.Dockerfile.  It attaches the sparse image files the
# host created to loop devices, formats them as XFS, mounts them, runs the
# ignored live tests and the benchmark matrix that the host compiled, and
# detaches everything again.  It is never invoked on the host directly.

set -euo pipefail

readonly IMAGE="${SOMA_XFS_IMAGE:-/scratch/soma-xfs-reflink.img}"
readonly TINY_IMAGE="${SOMA_XFS_TINY_IMAGE:-/scratch/soma-xfs-tiny.img}"
readonly NOREFLINK_IMAGE="${SOMA_XFS_NOREFLINK_IMAGE:-/scratch/soma-xfs-noreflink.img}"
readonly RAW_DIR="${SOMA_XFS_RAW_DIR:-/scratch/raw}"
readonly BENCH_BIN="${SOMA_XFS_BENCH_BIN:-/work/target/release/soma-storage-bench}"
readonly TEST_BINS="${SOMA_XFS_TEST_BINS:-}"
readonly BENCH_ARGS="${SOMA_XFS_BENCH_ARGS:-}"
readonly STAGE="${SOMA_XFS_STAGE:-all}"
readonly MOUNT_ROOT="/mnt/soma"

declare -a LOOPS=()
declare -a MOUNTS=()
ATTACHED_LOOP=""

log() {
    printf '\n==> %s\n' "$1"
}

cleanup() {
    local mount_point loop
    for mount_point in "${MOUNTS[@]}"; do
        umount "$mount_point" 2>/dev/null || true
    done
    for loop in "${LOOPS[@]}"; do
        losetup -d "$loop" 2>/dev/null || true
    done
}
trap cleanup EXIT

record_identity() {
    log "tool and kernel identity"
    uname -r
    dpkg-query -W -f='${Package} ${Version}\n' xfsprogs e2fsprogs util-linux coreutils
    mkfs.xfs -V
    mke2fs -V 2>&1 | sed -n '1p'
    {
        printf 'kernel %s\n' "$(uname -r)"
        dpkg-query -W -f='${Package} ${Version}\n' xfsprogs e2fsprogs util-linux coreutils
    } > "${RAW_DIR}/container-identity.txt"
}

# Docker populates a static /dev, so a loop device allocated through
# /dev/loop-control may have no node yet; create the missing block nodes.
ensure_loop_nodes() {
    local index
    for index in $(seq 0 63); do
        if [[ ! -e "/dev/loop${index}" ]]; then
            mknod "/dev/loop${index}" b 7 "$index"
        fi
    done
}

# attach_and_mount <image> <mount point> <label> <reflink 0|1>
# Publishes the loop device name through ATTACHED_LOOP so set -e still applies.
attach_and_mount() {
    local image="$1" mount_point="$2" label="$3" reflink="$4" loop
    ensure_loop_nodes
    loop="$(losetup --find --show --direct-io=on "$image")"
    LOOPS+=("$loop")
    mkfs.xfs -f -q -m "reflink=${reflink}" -L "$label" "$loop"
    mkdir -p "$mount_point"
    mount -t xfs -o noatime "$loop" "$mount_point"
    MOUNTS+=("$mount_point")
    printf '%s %s reflink=%s\n' "$loop" "$mount_point" "$reflink" >> "${RAW_DIR}/loop-map.txt"
    ATTACHED_LOOP="$loop"
}

prepare_filesystems() {
    log "attach loop devices and format XFS"
    mkdir -p "$RAW_DIR"
    : > "${RAW_DIR}/loop-map.txt"
    attach_and_mount "$IMAGE" "${MOUNT_ROOT}/reflink" soma-reflink 1
    MAIN_LOOP="$ATTACHED_LOOP"
    attach_and_mount "$TINY_IMAGE" "${MOUNT_ROOT}/tiny" soma-tiny 1
    TINY_LOOP="$ATTACHED_LOOP"
    attach_and_mount "$NOREFLINK_IMAGE" "${MOUNT_ROOT}/noreflink" soma-norefl 0
    NOREFLINK_LOOP="$ATTACHED_LOOP"
    readonly MAIN_LOOP TINY_LOOP NOREFLINK_LOOP
    xfs_info "${MOUNT_ROOT}/reflink" | tee "${RAW_DIR}/xfs-info-reflink.txt"
    xfs_info "${MOUNT_ROOT}/noreflink" > "${RAW_DIR}/xfs-info-noreflink.txt"
    grep -F "${MOUNT_ROOT}/" /proc/self/mountinfo | tee "${RAW_DIR}/mountinfo.txt"
    losetup --list --output NAME,BACK-FILE,DIO,LOG-SEC "$MAIN_LOOP" | tee "${RAW_DIR}/losetup.txt"
    mkdir -p "${MOUNT_ROOT}/reflink/heads" "${MOUNT_ROOT}/reflink/templates" \
        "${MOUNT_ROOT}/tiny/heads" "${MOUNT_ROOT}/noreflink/heads"
}

probe_reflink() {
    log "shell reflink probe"
    head -c 1048576 /dev/urandom > "${MOUNT_ROOT}/reflink/probe-a"
    cp --reflink=always "${MOUNT_ROOT}/reflink/probe-a" "${MOUNT_ROOT}/reflink/probe-b"
    cmp "${MOUNT_ROOT}/reflink/probe-a" "${MOUNT_ROOT}/reflink/probe-b"
    if cp --reflink=always "${MOUNT_ROOT}/reflink/probe-a" "${MOUNT_ROOT}/noreflink/probe-b" 2>/dev/null; then
        printf 'reflink unexpectedly succeeded on the reflink=0 filesystem\n' >&2
        return 1
    fi
    rm -f "${MOUNT_ROOT}/reflink/probe-a" "${MOUNT_ROOT}/reflink/probe-b"
    printf 'reflink works on %s and %s and is refused on %s\n' \
        "$MAIN_LOOP" "$TINY_LOOP" "$NOREFLINK_LOOP"
}

run_live_tests() {
    local test_bin
    if [[ -z "$TEST_BINS" ]]; then
        printf 'SOMA_XFS_TEST_BINS is empty; no live tests to run\n' >&2
        return 1
    fi
    log "ignored live tests on the loop-backed XFS"
    : > "${RAW_DIR}/live-tests.log"
    for test_bin in $TEST_BINS; do
        printf '%s\n' "$test_bin"
        SOMA_XFS_REFLINK_DIR="${MOUNT_ROOT}/reflink/heads" \
        SOMA_XFS_TINY_DIR="${MOUNT_ROOT}/tiny/heads" \
        SOMA_XFS_NOREFLINK_DIR="${MOUNT_ROOT}/noreflink/heads" \
        SOMA_XFS_TEMPLATE_DIR="${MOUNT_ROOT}/reflink/templates" \
            "$test_bin" --ignored --test-threads=1 2>&1 | tee -a "${RAW_DIR}/live-tests.log"
    done
}

remount_and_compare_templates() {
    log "remount and compare template digests"
    local before after
    before="$(cd "${MOUNT_ROOT}/reflink/templates" && sha256sum ./* | sort)"
    sync
    umount "${MOUNT_ROOT}/reflink"
    mount -t xfs -o noatime "$MAIN_LOOP" "${MOUNT_ROOT}/reflink"
    after="$(cd "${MOUNT_ROOT}/reflink/templates" && sha256sum ./* | sort)"
    if [[ "$before" != "$after" ]]; then
        printf 'template digests changed across remount\n%s\n%s\n' "$before" "$after" >&2
        return 1
    fi
    printf '%s\n' "$after" | tee "${RAW_DIR}/template-digests-after-remount.txt"
}

run_benchmark() {
    log "benchmark matrix"
    if [[ ! -x "$BENCH_BIN" ]]; then
        printf 'benchmark executable missing: %s\n' "$BENCH_BIN" >&2
        return 1
    fi
    # The Markdown summary on standard output is kept beside the raw samples so the
    # evidence document can quote it; progress lines stay on standard error.
    # shellcheck disable=SC2086
    SOMA_XFS_BACKING_FILE="$IMAGE" \
    SOMA_XFS_LOOP_DEVICE="$MAIN_LOOP" \
        "$BENCH_BIN" \
            --dir "${MOUNT_ROOT}/reflink" \
            --out "${RAW_DIR}/xfs-reflink-samples.jsonl" \
            $BENCH_ARGS \
        | tee "${RAW_DIR}/xfs-reflink-report.md"
    sha256sum "${RAW_DIR}/xfs-reflink-samples.jsonl" | tee "${RAW_DIR}/xfs-reflink-samples.sha256"
}

main() {
    mkdir -p "$RAW_DIR"
    record_identity
    prepare_filesystems
    probe_reflink
    case "$STAGE" in
        probe)
            ;;
        tests)
            run_live_tests
            remount_and_compare_templates
            ;;
        bench)
            run_benchmark
            ;;
        all)
            run_live_tests
            remount_and_compare_templates
            run_benchmark
            ;;
        *)
            printf 'unknown stage: %s\n' "$STAGE" >&2
            return 2
            ;;
    esac
    log "container run complete"
}

main "$@"
