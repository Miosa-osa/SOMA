#!/usr/bin/env bash
#
# Compiles a desktop image (workloads/desktop) into a captured, launchable Generation.
#
# A desktop Generation is an ordinary SOMA Generation: the same guest agent and devices as every
# other, on the desktop kernel variant (version 1 plus input and uinput), at a desktop shape,
# with a network device so a guest port (the viewer) can be tunnelled to.
# This script does the three steps that turn a local image into one, and refuses to stop halfway:
#
#   1. save the image from the local Docker daemon to an archive (skopeo cannot read the daemon of
#      a current Docker, so the archive is the exchange format; see scripts/stepa-host11.sh),
#   2. compile it with scripts/prepare-generation.sh at the shape, with a network device,
#   3. capture its snapshot at the same memory size, which certifies and promotes the Generation.
#
# Run it as root from the repository root on the KVM host, after build-soma.sh, build-fs-tools.sh
# and `cargo build --release -p soma-generation --example prepare_generation -p soma-local
# --example capture_snapshot`.
#
# Usage:
#   scripts/desktop-generation.sh <local-image> <reference> <store-dir> <fs-tools-dir> [shape]
#
# The reference names the entry (soma-api launches it by that name); the image is only its source.
# Shapes are the platform's computer sizes that fit the version 2 machine contract and a writable
# size the version 2 compiler profile builds:
#   small   2 vCPUs, 4096 MiB, 20480 MiB writable (default)
#   medium  4 vCPUs, 8192 MiB, 20480 MiB writable
#
# Example:
#   scripts/desktop-generation.sh soma-desktop:2 soma-desktop:small-2 /srv/soma/prepared /srv/soma/fs-tools

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly REPO_ROOT
cd "$REPO_ROOT"

IMAGE="${1:?usage: desktop-generation.sh <local-image> <reference> <store-dir> <fs-tools-dir> [shape]}"
REFERENCE="${2:?reference}"
STORE="${3:?store directory}"
FS_TOOLS="${4:?fs-tools directory}"
SHAPE="${5:-small}"
ARCHIVE=""

die() { printf 'desktop-generation: %s\n' "$1" >&2; exit 1; }
log() { printf '\n==> %s\n' "$1"; }

cleanup() {
    if [[ -n "$ARCHIVE" && -f "$ARCHIVE" ]]; then
        rm -f -- "$ARCHIVE"
    fi
}
trap cleanup EXIT

case "$SHAPE" in
    small) VCPUS=2 MEMORY_MIB=4096 STORAGE_MIB=20480 ;;
    medium) VCPUS=4 MEMORY_MIB=8192 STORAGE_MIB=20480 ;;
    *) die "unknown shape $SHAPE (small or medium)" ;;
esac

readonly CAPTURE_BIN="$REPO_ROOT/target/release/examples/capture_snapshot"
[[ -x "$CAPTURE_BIN" ]] || die "no capture tool; build it: cargo build --release -p soma-local --example capture_snapshot"
command -v docker >/dev/null || die "docker is required to read the local image"
# The desktop binds the desktop kernel variant (input and uinput, for the viewer), not version 1.
KERNEL="${SOMA_KERNEL:-$(compgen -G "kernel/out-desktop/vmlinux-*-soma-desktop" | head -1 || true)}"
[[ -n "$KERNEL" && -f "$KERNEL" ]] || die "no desktop kernel; run kernel/build-variant.sh desktop first"
docker image inspect "$IMAGE" >/dev/null 2>&1 || die "no local image $IMAGE; build workloads/desktop first"
docker run --rm --network none --entrypoint /bin/sh "$IMAGE" -c \
    'test -x /usr/local/sbin/soma-desktop-session && test -x /opt/cua-driver/cua-driver' ||
    die "$IMAGE is not a desktop image: it has no soma-desktop-session or cua-driver"

mkdir -p "$STORE"
ENTRY="$STORE/ref-$(printf '%s' "$REFERENCE" | sha256sum | cut -d' ' -f1)"
[[ ! -e "$ENTRY" ]] || die "$ENTRY already exists; a new image needs a new reference"

log "saving $IMAGE to an archive"
ARCHIVE="$(mktemp "$STORE/.desktop-image.XXXXXXXX.tar")"
docker save -o "$ARCHIVE" "$IMAGE"

log "compiling $REFERENCE at $SHAPE ($VCPUS vCPUs, $MEMORY_MIB MiB, $STORAGE_MIB MiB writable)"
# The capture below is what makes this entry restore rather than cold boot, so the prepare step is
# allowed to end on a cold-boot-only entry and this script is the one that refuses to.
SOMA_KERNEL="$KERNEL" \
SOMA_IMAGE_SOURCE="docker-archive:$ARCHIVE" \
SOMA_PREPARE_VCPUS="$VCPUS" \
SOMA_PREPARE_NETWORK=public_internet \
SOMA_ALLOW_COLD_BOOT_ENTRY=1 \
    ./scripts/prepare-generation.sh "$REFERENCE" "$STORE" "$FS_TOOLS" "$MEMORY_MIB" "$STORAGE_MIB"
rm -f -- "$ARCHIVE"
ARCHIVE=""

log "capturing the snapshot at $MEMORY_MIB MiB"
"$CAPTURE_BIN" "$ENTRY" "$MEMORY_MIB"
[[ -f "$ENTRY/snapshot/state.somasnap" && -s "$ENTRY/generation.id" ]] ||
    die "the capture did not certify $ENTRY; it would cold boot"
grep -q 'soma.net=eth0' <(tr -c '[:print:]' '\n' <"$ENTRY/candidate.somacan") ||
    die "$ENTRY has no network device, so no guest port could ever be tunnelled to"

log "done"
printf '  reference:  %s\n  entry:      %s\n  generation: %s\n  shape:      %s vCPUs, %s MiB, %s MiB writable\n' \
    "$REFERENCE" "$ENTRY" "$(cat "$ENTRY/generation.id")" "$VCPUS" "$MEMORY_MIB" "$STORAGE_MIB"
printf '  lower:      %s\n' "$(du -sh "$ENTRY/store" | cut -f1)"
