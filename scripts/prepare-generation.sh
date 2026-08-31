#!/usr/bin/env bash
#
# Compiles one OCI image into a launchable Generation entry in a prepared store.
#
# This is the primitive that turns "build before demand" into a command. It exports the image to
# an OCI layout with skopeo (which produces a real OCI layout regardless of the local docker image
# store), then runs the prepare_generation tool with the built kernel, guest agent, and the pinned
# filesystem tools, writing one entry the KVM backend can resolve and launch.
#
# Run it from the repository root, after build-soma.sh and build-fs-tools.sh.
#
# Usage:
#   prepare-generation.sh <image-reference> <store-directory> <fs-tools-directory> \
#       [memory_mib] [storage_mib]
#
# Example:
#   scripts/prepare-generation.sh node:22 /srv/soma/prepared /srv/soma/fs-tools
#
# A compiled entry with no snapshot still launches: it cold boots a kernel instead of restoring
# a captured machine, which is roughly fifteen times slower and is reported nowhere at launch.
# This script therefore refuses to end quietly on such an entry. Set
# SOMA_ALLOW_COLD_BOOT_ENTRY=1 to accept one deliberately.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
cd "$REPO_ROOT"

IMAGE=""
STORE_DIR=""
FS_TOOLS=""
MEM_MIB=""
DISK_MIB=""

readonly MUSL_TARGET="x86_64-unknown-linux-musl"
readonly AGENT_BIN="$REPO_ROOT/target/${MUSL_TARGET}/release/soma-guest-agent"
readonly PREPARE_BIN="$REPO_ROOT/target/release/examples/prepare_generation"
LAYOUT=""

log() { printf '\n==> %s\n' "$1"; }
die() { printf '%s\n' "$1" >&2; exit 1; }

parse_args() {
    if [[ $# -lt 3 || $# -gt 5 ]]; then
        printf 'usage: prepare-generation.sh <image> <store-dir> <fs-tools-dir> [mem_mib] [disk_mib]\n' >&2
        return 2
    fi
    IMAGE="$1"
    STORE_DIR="$(mkdir -p "$2" && cd "$2" && pwd -P)"
    FS_TOOLS="$(cd "$3" && pwd -P)"
    MEM_MIB="${4:-1024}"
    DISK_MIB="${5:-10240}"
}

cleanup() {
    if [[ -n "$LAYOUT" && -d "$LAYOUT" ]]; then
        rm -rf -- "$LAYOUT"
    fi
}
trap cleanup EXIT

registry_reference() {
    local first="${IMAGE%%/*}"
    if [[ "$IMAGE" != */* ]]; then
        printf 'docker.io/library/%s\n' "$IMAGE"
    elif [[ "$first" == *.* || "$first" == *:* || "$first" == "localhost" ]]; then
        printf '%s\n' "$IMAGE"
    else
        printf 'docker.io/%s\n' "$IMAGE"
    fi
}

reference_key() {
    printf '%s' "$IMAGE" | sha256sum | cut -d' ' -f1
}

resolve_inputs() {
    KERNEL="$(compgen -G "$REPO_ROOT/kernel/out/vmlinux-*-soma-v1" | head -1 || true)"
    [[ -n "$KERNEL" ]] || die "no built kernel under kernel/out; run build-soma.sh first"
    KERNEL_CONFIG="$(dirname "$KERNEL")/final.config"
    [[ -f "$KERNEL_CONFIG" ]] || KERNEL_CONFIG="$REPO_ROOT/kernel/config-x86_64-soma-v1"
    [[ -f "$KERNEL_CONFIG" ]] || die "kernel configuration not found beside the kernel"
    [[ -f "$AGENT_BIN" ]] || die "no guest agent; run build-soma.sh first"
    [[ -d "$FS_TOOLS/erofs" && -d "$FS_TOOLS/e2fsprogs" ]] || \
        die "fs-tools dir must have erofs/ and e2fsprogs/; run build-fs-tools.sh first"
    if [[ ! -x "$PREPARE_BIN" ]]; then
        log "building the prepare tool"
        cargo build --release -p soma-generation --example prepare_generation >/dev/null
    fi
}

export_oci() {
    local source
    source="$(registry_reference)"
    LAYOUT="$(mktemp -d "$STORE_DIR/.oci.XXXXXXXX")"
    log "exporting $IMAGE to an OCI layout with skopeo"
    command -v skopeo >/dev/null || die "skopeo is required; setup-host.sh installs it"
    # docker:// pulls from the registry without needing a local docker image or the docker daemon.
    skopeo copy --override-os linux --override-arch amd64 \
        "docker://$source" "oci:$LAYOUT:soma"
    [[ -f "$LAYOUT/oci-layout" ]] || die "skopeo did not produce an OCI layout"
}

prepare() {
    local entry
    entry="$STORE_DIR/ref-$(reference_key)"
    log "compiling the Generation into $entry"
    "$PREPARE_BIN" "$IMAGE" "$LAYOUT" "$KERNEL" "$KERNEL_CONFIG" "$AGENT_BIN" \
        "$FS_TOOLS/erofs" "$FS_TOOLS/e2fsprogs" "$entry" "$MEM_MIB" "$DISK_MIB"
    rm -rf -- "$LAYOUT"
    LAYOUT=""
    log "done"
    printf '  entry:  %s\n  launch: SOMA_GENERATION_STORE=%s SOMA_ALLOW_UNCERTIFIED_GENERATION=1 \\\n' \
        "$entry" "$STORE_DIR"
    printf '          soma --backend kvm run --memory-mib %s --storage-mib %s %s -- <command>\n' \
        "$MEM_MIB" "$DISK_MIB" "$IMAGE"
    printf '  shape:  a snapshot restores only the memory it was captured at; launching this\n'
    printf '          entry at any other --memory-mib is refused as backend_unavailable\n'

    report_boot_path "$entry"
}

# The path a launch from this entry will actually take, said out loud.
#
# The backend chooses restore over cold boot by the presence of one file and reports neither
# choice, so an entry with no snapshot produces a measurement that looks exactly like a working
# one and is an order of magnitude slower. Silence here is the whole defect.
snapshot_state_file() {
    printf '%s/snapshot/state.somasnap' "$1"
}

report_boot_path() {
    local entry="$1"
    local state
    state="$(snapshot_state_file "$entry")"
    if [[ -f "$state" ]]; then
        printf '  boot:   RESTORE from %s/snapshot\n' "$entry"
        return 0
    fi
    printf '  boot:   COLD BOOT; this entry has no snapshot\n'
    {
        printf '\n'
        printf 'COLD BOOT ONLY: %s does not exist.\n' "$state"
        printf 'A launch from this entry boots a kernel instead of restoring a captured\n'
        printf 'machine. That is roughly fifteen times slower, and nothing at launch says\n'
        printf 'which path was taken, so a measurement against this entry is indistinguishable\n'
        printf 'from a working one. Capture a snapshot before measuring anything:\n\n'
        printf '  ./target/release/examples/capture_snapshot %s %s\n\n' "$entry" "$MEM_MIB"
        printf 'Set SOMA_ALLOW_COLD_BOOT_ENTRY=1 to accept a cold boot only entry on purpose.\n'
    } >&2
    if [[ "${SOMA_ALLOW_COLD_BOOT_ENTRY:-0}" == "1" ]]; then
        printf 'SOMA_ALLOW_COLD_BOOT_ENTRY=1: accepted a cold boot only entry.\n' >&2
        return 0
    fi
    return 3
}

main() {
    parse_args "$@"
    resolve_inputs
    export_oci
    prepare
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
