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
#       [memory_mib] [storage_mib] [ceiling_mib]
#
# A ceiling_mib (or SOMA_PREPARE_CEILING_MIB, which the argument wins over) builds the live-resize
# machine: a compiler profile version 3 Generation with a memory device whose manifest declares a
# hotplug region of ceiling_mib minus memory_mib, so a running sandbox can be grown up to
# ceiling_mib and shrunk back to memory_mib. It must be above memory_mib. Without it the
# Generation has no memory device and cannot be resized in memory.
#
# The reference names the entry (it is hashed into the entry's directory), and by default it is
# also where the image is pulled from. An image built on this machine and never pushed, such as
# workloads/browser-default, is exported from the local docker daemon instead by naming the
# skopeo source explicitly:
#
#   SOMA_IMAGE_SOURCE=docker-daemon:soma-browser-default:1 \
#       scripts/prepare-generation.sh soma-browser-default:1 /srv/soma/prepared /srv/soma/fs-tools 2048
#
# Example:
#   scripts/prepare-generation.sh node:22 /srv/soma/prepared /srv/soma/fs-tools
#
# A compiled entry with no snapshot still launches: it cold boots a kernel instead of restoring
# a captured machine, which is roughly fifteen times slower and is reported nowhere at launch.
# This script therefore refuses to end quietly on such an entry. Set
# SOMA_ALLOW_COLD_BOOT_ENTRY=1 to accept one deliberately.

# SOMA_CAPTURE_WARM declares a capture warm plan: commands separated by ';' that the guest agent
# runs once, at the disconnected repair point and before the snapshot is taken. Their pages are
# then part of the captured memory, so every Instance restored from this entry finds them
# resident rather than paying the first touch itself. Name the runtime a sandbox will be asked
# for, which is the one whose first command matters:
#
#   SOMA_CAPTURE_WARM='/usr/local/bin/node -v' \
#       scripts/prepare-generation.sh node:22 /srv/soma/prepared /srv/soma/fs-tools
#
# This is the capture-time counterpart of the host's SOMA_LAUNCH_WARM_COMMAND, and it is the one
# that costs a create nothing: the plan is paid once here, instead of once per launch. It costs
# capture time - up to ten seconds per command, plus a two second settle before the capture
# point - and it is advisory: a command the image does not have is reported and killed, and the
# entry still builds. The plan is part of the Generation, written into the initramfs whose digest
# the manifest binds, so changing it changes the entry's identity.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
cd "$REPO_ROOT"

IMAGE=""
STORE_DIR=""
FS_TOOLS=""
MEM_MIB=""
DISK_MIB=""
CEILING_MIB=""
WARM_PLAN=""

readonly MUSL_TARGET="x86_64-unknown-linux-musl"
readonly AGENT_BIN="$REPO_ROOT/target/${MUSL_TARGET}/release/soma-guest-agent"
readonly PREPARE_BIN="$REPO_ROOT/target/release/examples/prepare_generation"
LAYOUT=""

log() { printf '\n==> %s\n' "$1"; }
die() { printf '%s\n' "$1" >&2; exit 1; }

parse_args() {
    if [[ $# -lt 3 || $# -gt 6 ]]; then
        printf 'usage: prepare-generation.sh <image> <store-dir> <fs-tools-dir> [mem_mib] [disk_mib] [ceiling_mib]\n' >&2
        return 2
    fi
    IMAGE="$1"
    STORE_DIR="$(mkdir -p "$2" && cd "$2" && pwd -P)"
    FS_TOOLS="$(cd "$3" && pwd -P)"
    MEM_MIB="${4:-1024}"
    DISK_MIB="${5:-10240}"
    CEILING_MIB="${6:-${SOMA_PREPARE_CEILING_MIB:-}}"
    WARM_PLAN="${SOMA_CAPTURE_WARM:-}"
    if [[ -n "$CEILING_MIB" ]]; then
        [[ "$CEILING_MIB" =~ ^[0-9]+$ && "$MEM_MIB" =~ ^[0-9]+$ ]] || \
            die "ceiling_mib and mem_mib must be positive integers, got ceiling '$CEILING_MIB' memory '$MEM_MIB'"
        (( CEILING_MIB > MEM_MIB )) || \
            die "ceiling_mib ($CEILING_MIB) must be above mem_mib ($MEM_MIB): the difference is the hotplug region"
    fi
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
    local source="${SOMA_IMAGE_SOURCE:-docker://$(registry_reference)}"
    LAYOUT="$(mktemp -d "$STORE_DIR/.oci.XXXXXXXX")"
    log "exporting $IMAGE to an OCI layout with skopeo"
    command -v skopeo >/dev/null || die "skopeo is required; setup-host.sh installs it"
    # docker:// pulls from the registry without needing a local docker image or the docker daemon.
    skopeo copy --override-os linux --override-arch amd64 \
        "$source" "oci:$LAYOUT:soma"
    [[ -f "$LAYOUT/oci-layout" ]] || die "skopeo did not produce an OCI layout"
}

prepare() {
    local entry
    entry="$STORE_DIR/ref-$(reference_key)"
    log "compiling the Generation into $entry"
    if [[ -n "$CEILING_MIB" ]]; then
        log "live-resize machine: boot ${MEM_MIB} MiB, ceiling ${CEILING_MIB} MiB (hotplug region $((CEILING_MIB - MEM_MIB)) MiB)"
    fi
    if [[ -n "$WARM_PLAN" ]]; then
        log "capture warm plan: $WARM_PLAN"
    fi
    # Passed explicitly, set or empty, so an inherited value never reaches the tool unseen.
    SOMA_PREPARE_CEILING_MIB="$CEILING_MIB" SOMA_CAPTURE_WARM="$WARM_PLAN" \
        "$PREPARE_BIN" "$IMAGE" "$LAYOUT" "$KERNEL" \
        "$KERNEL_CONFIG" "$AGENT_BIN" "$FS_TOOLS/erofs" "$FS_TOOLS/e2fsprogs" "$entry" \
        "$MEM_MIB" "$DISK_MIB"
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
