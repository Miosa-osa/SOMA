#!/usr/bin/env bash
#
# Produces the exact filesystem tools the Generation compiler pins and verifies.
#
# The compiler rejects any other version: it requires erofs-utils 1.9.4 and e2fsprogs 1.47.0 and
# checks their reported revisions before it will build a root or an overlay. A distribution
# erofs-utils is almost never 1.9.4, so it is built from the pinned source inside a container. On
# Ubuntu 24.04 the distribution e2fsprogs is already 1.47.0, so its formatter is used in place.
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
readonly BUILDER="ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517"

OUT="${1:-fs-tools}"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd -P)"
readonly OUT

log() { printf '\n==> %s\n' "$1"; }

build_erofs() {
    log "building erofs-utils $EROFS_VERSION from the pinned source (in a container)"
    mkdir -p "$OUT/erofs"
    docker run --rm -v "$OUT/erofs:/out" -e "EROFS_COMMIT=$EROFS_COMMIT" "$BUILDER" \
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

arrange_e2fsprogs() {
    log "checking the distribution e2fsprogs is $E2FSPROGS_VERSION"
    local formatter
    formatter="$(command -v mke2fs || echo /usr/sbin/mke2fs)"
    if [[ ! -x "$formatter" ]]; then
        printf 'mke2fs not found; install e2fsprogs first (setup-host.sh does this)\n' >&2
        exit 1
    fi
    local reported
    reported="$("$formatter" -V 2>&1 | head -1 | awk '{print $2}')"
    if [[ "$reported" != "$E2FSPROGS_VERSION" ]]; then
        printf 'e2fsprogs is %s, the compiler pins %s. Build 1.47.0 from source and point\n' \
            "$reported" "$E2FSPROGS_VERSION" >&2
        printf 'SOMA_E2FSPROGS at it. Ubuntu 24.04 ships 1.47.0, so this host is unexpected.\n' >&2
        exit 1
    fi
    mkdir -p "$OUT/e2fsprogs"
    # A clean directory holding just the formatter, so the compiler is not handed a system bin
    # directory full of unrelated tools.
    for tool in mke2fs mkfs.ext4 dumpe2fs; do
        local path
        path="$(command -v "$tool" || echo "/usr/sbin/$tool")"
        [[ -x "$path" ]] && ln -sf "$path" "$OUT/e2fsprogs/$tool"
    done
    printf '  e2fsprogs %s at %s/e2fsprogs/\n' "$reported" "$OUT"
}

main() {
    command -v docker >/dev/null || { printf 'docker is required\n' >&2; exit 1; }
    build_erofs
    arrange_e2fsprogs
    log "filesystem tools ready"
    printf '  SOMA_EROFS_TOOLS=%s/erofs\n  SOMA_E2FSPROGS=%s/e2fsprogs\n' "$OUT" "$OUT"
}

main "$@"
