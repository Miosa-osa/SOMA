#!/usr/bin/env bash
# Host side of the XFS reflink measurement for decision-map ticket #11.
#
# The host root filesystem does not need reflink support and the host needs no
# root privileges beyond `docker run --privileged`.  The script builds the
# benchmark and live-test executables on the host, creates sparse image files
# on the host's storage, and runs scripts/xfs-reflink-container.sh inside the
# pinned Ubuntu 24.04 image so that xfsprogs, e2fsprogs, losetup, the loop-backed
# XFS mounts, and drop_caches all happen inside the container.
#
# Raw JSONL samples land in SOMA_XFS_SCRATCH/raw (default target/xfs-reflink/raw)
# and are not committed; the evidence document records their SHA-256.
#
# usage: scripts/xfs-reflink-bench.sh [probe|tests|bench|all] [extra bench args]
#   SOMA_XFS_SCRATCH   scratch directory for images and raw output
#   SOMA_XFS_IMAGE_GIB size of the main XFS image in GiB (default 20)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd -P)"
readonly REPO_ROOT
readonly STAGE="${1:-all}"
shift || true
readonly SCRATCH="${SOMA_XFS_SCRATCH:-${REPO_ROOT}/target/xfs-reflink}"
readonly IMAGE_GIB="${SOMA_XFS_IMAGE_GIB:-20}"
readonly IMAGE_TAG="soma-xfs-bench"
readonly BASE_IMAGE="ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517"

cd "${REPO_ROOT}"

log() {
    printf '\n==> %s\n' "$1"
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'required command not found: %s\n' "$1" >&2
        return 1
    fi
}

check_host() {
    require_command docker
    require_command cargo
    require_command python3
    if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
        printf 'the XFS reflink measurement requires a Linux x86_64 host\n' >&2
        return 1
    fi
    if ! grep -q "^FROM ${BASE_IMAGE}\$" "${SCRIPT_DIR}/xfs-reflink-bench.Dockerfile"; then
        printf 'xfs-reflink-bench.Dockerfile is not pinned to the expected base digest\n' >&2
        return 1
    fi
}

build_executables() {
    log "build benchmark and live-test executables"
    cargo build --locked --release -p soma-storage --bin soma-storage-bench
    TEST_BINS="$(
        cargo test --locked --release -p soma-storage --no-run --message-format=json 2>/dev/null \
            | python3 -c '
import json, sys
for line in sys.stdin:
    try:
        event = json.loads(line)
    except ValueError:
        continue
    if event.get("reason") == "compiler-artifact" and event.get("executable"):
        target = event["target"]
        if target["name"] in ("soma_storage", "soma-storage", "xfs_live"):
            print(event["executable"])
'
    )"
    readonly TEST_BINS
    if [[ -z "$TEST_BINS" ]]; then
        printf 'no test executables were produced\n' >&2
        return 1
    fi
    printf '%s\n' "$TEST_BINS"
}

# Creates a sparse file of the requested size without touching the host root
# filesystem's reflink capability.
create_sparse_image() {
    local path="$1" bytes="$2"
    rm -f "$path"
    python3 - "$path" "$bytes" <<'EOF'
import sys
path, size = sys.argv[1], int(sys.argv[2])
with open(path, "xb") as image:
    image.seek(size - 1)
    image.write(b"\0")
EOF
}

prepare_images() {
    log "create sparse XFS image files under ${SCRATCH}"
    mkdir -p "${SCRATCH}/raw"
    create_sparse_image "${SCRATCH}/soma-xfs-reflink.img" $(( IMAGE_GIB * 1024 * 1024 * 1024 ))
    create_sparse_image "${SCRATCH}/soma-xfs-tiny.img" $(( 320 * 1024 * 1024 ))
    create_sparse_image "${SCRATCH}/soma-xfs-noreflink.img" $(( 320 * 1024 * 1024 ))
    rm -f "${SCRATCH}/raw/xfs-reflink-samples.jsonl" "${SCRATCH}/raw/xfs-reflink-samples.sha256"
    /bin/ls -ls "${SCRATCH}"/*.img
}

build_container_image() {
    log "build the pinned measurement image"
    docker build -q -t "${IMAGE_TAG}" -f "${SCRIPT_DIR}/xfs-reflink-bench.Dockerfile" "${SCRIPT_DIR}"
    docker image inspect --format '{{.Id}}' "${IMAGE_TAG}" | tee "${SCRATCH}/raw/container-image-id.txt"
}

container_paths() {
    local host_path
    for host_path in $TEST_BINS; do
        printf '/work/%s ' "${host_path#"${REPO_ROOT}"/}"
    done
}

run_container() {
    local git_rev
    git_rev="$(git rev-parse HEAD 2>/dev/null || printf 'unknown')"
    if [[ -n "$(git status --porcelain 2>/dev/null)" ]]; then
        git_rev="${git_rev}-dirty"
    fi
    log "run stage ${STAGE} inside the privileged container"
    docker run --rm --privileged \
        -v "${REPO_ROOT}:/work:ro" \
        -v "${SCRATCH}:/scratch" \
        -e SOMA_XFS_STAGE="${STAGE}" \
        -e SOMA_XFS_TEST_BINS="$(container_paths)" \
        -e SOMA_XFS_BENCH_ARGS="$*" \
        -e SOMA_GIT_REV="${git_rev}" \
        "${IMAGE_TAG}" /work/scripts/xfs-reflink-container.sh \
        2>&1 | tee "${SCRATCH}/raw/container-${STAGE}.log"
}

main() {
    case "$STAGE" in
        probe|tests|bench|all) ;;
        *)
            printf 'usage: %s [probe|tests|bench|all] [extra bench args]\n' "$0" >&2
            return 2
            ;;
    esac
    check_host
    build_executables
    prepare_images
    build_container_image
    run_container "$@"
    log "raw output"
    /bin/ls -l "${SCRATCH}/raw"
}

main "$@"
