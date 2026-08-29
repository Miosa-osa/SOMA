#!/usr/bin/env bash
set -euo pipefail

# PROTOTYPE ONLY.
# This proves that the pinned EROFS recipe produces identical bytes from
# logically identical trees created in different host insertion orders.

readonly builder='ubuntu@sha256:561618e2c15bf2397621dd04f96926663a3b5616c189cf7e38db7e82f5c538ea'
readonly erofs_revision='f36cadb5c563995ab3aa8572a60ed6b721b9557d'

docker run --rm \
    --env "EROFS_REVISION=${erofs_revision}" \
    "${builder}" \
    sh -euxc '
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq \
    autoconf \
    automake \
    build-essential \
    git \
    liblz4-dev \
    liblzma-dev \
    libtool \
    libzstd-dev \
    pkg-config \
    uuid-dev >/dev/null

git clone -q https://github.com/erofs/erofs-utils.git /tmp/erofs
git -C /tmp/erofs checkout -q --detach "$EROFS_REVISION"
cd /tmp/erofs
./autogen.sh >/dev/null
./configure --disable-fuse >/dev/null
make -j2 >/dev/null

readonly mkfs=/tmp/erofs/mkfs/mkfs.erofs
readonly fsck=/tmp/erofs/fsck/fsck.erofs
readonly epoch=1700000000
readonly uuid=11111111-2222-4333-8444-555555555555

normalize_metadata() {
    root=$1
    chmod 0755 "$root"
    find "$root" -exec touch -h -d "@$epoch" {} +
}

create_forward() {
    root=$1
    mkdir -p "$root/etc" "$root/usr/local/bin" "$root/tmp"
    printf alpha >"$root/etc/a"
    printf zulu >"$root/etc/z"
    printf "#!/bin/sh\necho ready\n" >"$root/usr/local/bin/soma-agent"
    ln "$root/etc/a" "$root/etc/a-hard"
    ln -s ../etc/a "$root/a-link"
    mkfifo "$root/pipe"
    chmod 0755 "$root/usr/local/bin/soma-agent"
    chmod 1777 "$root/tmp"
    normalize_metadata "$root"
}

create_reverse() {
    root=$1
    mkdir -p "$root/tmp" "$root/usr/local/bin" "$root/etc"
    mkfifo "$root/pipe"
    ln -s ../etc/a "$root/a-link"
    printf "#!/bin/sh\necho ready\n" >"$root/usr/local/bin/soma-agent"
    printf zulu >"$root/etc/z"
    printf alpha >"$root/etc/a"
    ln "$root/etc/a" "$root/etc/a-hard"
    chmod 0755 "$root/usr/local/bin/soma-agent"
    chmod 1777 "$root/tmp"
    normalize_metadata "$root"
}

compile() {
    root=$1
    output=$2
    "$mkfs" \
        -T "$epoch" \
        --all-time \
        -U "$uuid" \
        -L SOMA_ROOT \
        "$output" \
        "$root" >/dev/null
}

create_forward /tmp/forward
create_reverse /tmp/reverse
compile /tmp/forward /tmp/forward.erofs
compile /tmp/reverse /tmp/reverse.erofs

cmp /tmp/forward.erofs /tmp/reverse.erofs
"$fsck" /tmp/forward.erofs

actual=$(sha256sum /tmp/forward.erofs | cut -d " " -f 1)
printf "prototype=PASS\n"
printf "erofs_revision=%s\n" "$EROFS_REVISION"
printf "root_disk_sha256=%s\n" "$actual"
'
