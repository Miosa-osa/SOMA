#!/usr/bin/env bash
#
# Produces the exact filesystem tools the Generation compiler pins and verifies.
#
# The compiler rejects any other version: it requires erofs-utils 1.9.4 and e2fsprogs 1.47.0 and
# checks their reported revisions before it will build a root or an overlay.
# Both toolsets are built from pinned source inside a pinned container image.
#
# Output: a tools directory with an `erofs/` subdirectory holding the built erofs binaries and an
# `e2fsprogs/` subdirectory holding the ext formatter. Pass these to prepare-generation.sh, or set
# SOMA_EROFS_TOOLS and SOMA_E2FSPROGS to them.
#
# Usage: build-fs-tools.sh [output-directory]   (default: ./fs-tools)

set -euo pipefail

readonly EROFS_COMMIT="f36cadb5c563995ab3aa8572a60ed6b721b9557d"   # erofs-utils 1.9.4
readonly EROFS_VERSION="1.9.4"
readonly E2FSPROGS_VERSION="1.47.0"
readonly E2FSPROGS_URL="https://mirrors.edge.kernel.org/pub/linux/kernel/people/tytso/e2fsprogs/v1.47.0/e2fsprogs-1.47.0.tar.xz"
readonly E2FSPROGS_SHA256="144af53f2bbd921cef6f8bea88bb9faddca865da3fbc657cc9b4d2001097d5db"
readonly BUILDER="ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517"

OUT="${1:-fs-tools}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd -P)"
readonly OUT

log() { printf '\n==> %s\n' "$1"; }

build_erofs() {
    log "building erofs-utils $EROFS_VERSION from the pinned source (in a container)"
    mkdir -p "$OUT/erofs"
    docker run --rm --platform linux/amd64 -v "$OUT/erofs:/out" \
        -e "EROFS_COMMIT=$EROFS_COMMIT" "$BUILDER" \
        sh -euxc '
            export DEBIAN_FRONTEND=noninteractive
            apt-get update -qq
            apt-get install -y -qq autoconf automake build-essential git liblz4-dev \
                liblzma-dev libtool libzstd-dev pkg-config uuid-dev >/dev/null
            git clone -q https://github.com/erofs/erofs-utils.git /tmp/erofs
            git -C /tmp/erofs checkout -q --detach "$EROFS_COMMIT"
            cd /tmp/erofs
            ./autogen.sh >/dev/null
            ./configure --disable-fuse >/dev/null
            make -j"$(nproc)" >/dev/null
            cp mkfs/mkfs.erofs fsck/fsck.erofs dump/dump.erofs /out/
        '
    printf '  built %s\n' "$OUT/erofs/"*.erofs
}

build_e2fsprogs() {
    log "building e2fsprogs $E2FSPROGS_VERSION from its pinned source (in a container)"
    mkdir -p "$OUT/e2fsprogs"
    docker run --rm --platform linux/amd64 -v "$OUT/e2fsprogs:/out" \
        -e "SOURCE_URL=$E2FSPROGS_URL" -e "SOURCE_SHA256=$E2FSPROGS_SHA256" "$BUILDER" \
        sh -euxc '
            export DEBIAN_FRONTEND=noninteractive
            apt-get update -qq
            apt-get install -y -qq build-essential curl libblkid-dev libuuid1 uuid-dev xz-utils >/dev/null
            curl --proto "=https" --tlsv1.2 -fsSL "$SOURCE_URL" -o /tmp/e2fsprogs.tar.xz
            printf "%s  %s\n" "$SOURCE_SHA256" /tmp/e2fsprogs.tar.xz | sha256sum -c -
            mkdir /tmp/source
            tar -xJf /tmp/e2fsprogs.tar.xz -C /tmp/source --strip-components=1
            mkdir /tmp/build
            cd /tmp/build
            /tmp/source/configure --disable-nls --disable-elf-shlibs >/dev/null
            make -j"$(nproc)" >/dev/null
            cp misc/mke2fs misc/dumpe2fs e2fsck/e2fsck debugfs/debugfs /out/
            ln -s mke2fs /out/mkfs.ext4
        '
    printf '  built e2fsprogs tools at %s/e2fsprogs/\n' "$OUT"
}

main() {
    command -v docker >/dev/null || { printf 'docker is required\n' >&2; exit 1; }
    build_erofs
    build_e2fsprogs
    log "filesystem tools ready"
    printf '  SOMA_EROFS_TOOLS=%s/erofs\n  SOMA_E2FSPROGS=%s/e2fsprogs\n' "$OUT" "$OUT"
}

main "$@"
