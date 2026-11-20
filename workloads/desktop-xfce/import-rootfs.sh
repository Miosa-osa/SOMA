#!/usr/bin/env bash
#
# Turns an existing desktop root filesystem image (an ext4 file, such as the one a Firecracker
# desktop template boots from) into a local Docker image, so it can be the base the SOMA desktop
# stage builds on.
#
# The file is attached read only and without journal replay, so the source image is never written.
#
# The archive is written in GNU format with ownership and modes only. A SOMA Generation carries no
# extended attributes: its layer normalizer accepts no PAX record but path and linkpath, and GNU
# tar's POSIX format (which --xattrs selects) writes atime and ctime records for every entry, so
# such a layer is refused as Unsupported. File capabilities would therefore be lost, so the import
# refuses a root filesystem that has any rather than dropping them unseen.
#
# What a running system leaves in /tmp, /run, /var/tmp, /dev, /proc and /sys is left out: the guest
# agent gives every Instance fresh ones, and build leftovers there would only enlarge the lower.
#
# Usage (as root on a Linux host with Docker):
#   workloads/desktop-xfce/import-rootfs.sh <rootfs.ext4> <image-tag>
#
# Example:
#   workloads/desktop-xfce/import-rootfs.sh /srv/images/ubuntu-24.04-desktop.ext4 desktop-rootfs:v17

set -euo pipefail

SOURCE="${1:?usage: import-rootfs.sh <rootfs.ext4> <image-tag>}"
TAG="${2:?usage: import-rootfs.sh <rootfs.ext4> <image-tag>}"
MOUNT_POINT=""

die() { printf 'import-rootfs: %s\n' "$1" >&2; exit 1; }

cleanup() {
    if [[ -n "$MOUNT_POINT" ]]; then
        if mountpoint -q "$MOUNT_POINT"; then
            umount "$MOUNT_POINT"
        fi
        rmdir "$MOUNT_POINT"
    fi
}
trap cleanup EXIT

[[ "$(id -u)" == 0 ]] || die "attaching an ext4 image needs root"
[[ -f "$SOURCE" ]] || die "no such file: $SOURCE"
command -v docker >/dev/null || die "docker is required"

MOUNT_POINT="$(mktemp -d "${TMPDIR:-/tmp}/desktop-rootfs.XXXXXXXX")"
# noload: an image whose journal was not cleanly closed is read as it is, never replayed into.
mount -o ro,loop,noload "$SOURCE" "$MOUNT_POINT"
[[ -x "$MOUNT_POINT/bin/sh" || -L "$MOUNT_POINT/bin/sh" ]] || die "$SOURCE holds no /bin/sh; is it a root filesystem?"

if command -v getcap >/dev/null; then
    capabilities="$(getcap -r "$MOUNT_POINT" 2>/dev/null || true)"
    [[ -z "$capabilities" ]] || die "files with capabilities, which a Generation cannot carry: $capabilities"
fi

printf '==> importing %s (%s used) as %s\n' "$SOURCE" "$(df -h --output=used "$MOUNT_POINT" | tail -1 | tr -d ' ')" "$TAG"
tar --format=gnu --numeric-owner -C "$MOUNT_POINT" \
    --exclude='./tmp/*' --exclude='./run/*' --exclude='./var/tmp/*' \
    --exclude='./dev/*' --exclude='./proc/*' --exclude='./sys/*' \
    -cf - . | docker import - "$TAG" >/dev/null
docker image inspect --format '{{.Id}} {{.Size}}' "$TAG"
