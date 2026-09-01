#!/usr/bin/env bash
# Prove that SOMA's prepared artifacts and private heads share one reflink XFS device.

set -euo pipefail

readonly PREPARED_DIR="${1:?usage: check-fast-storage.sh PREPARED_DIR HEAD_DIR}"
readonly HEAD_DIR="${2:?usage: check-fast-storage.sh PREPARED_DIR HEAD_DIR}"

for directory in "$PREPARED_DIR" "$HEAD_DIR"; do
    if [[ ! -d "$directory" || ! -w "$directory" ]]; then
        printf 'not a writable directory: %s\n' "$directory" >&2
        exit 1
    fi
done

prepared_device="$(findmnt -T "$PREPARED_DIR" -nro MAJ:MIN)"
head_device="$(findmnt -T "$HEAD_DIR" -nro MAJ:MIN)"
filesystem="$(findmnt -T "$HEAD_DIR" -nro FSTYPE)"
if [[ "$prepared_device" != "$head_device" ]]; then
    printf 'prepared artifacts and heads are on different devices: %s != %s\n' \
        "$prepared_device" "$head_device" >&2
    exit 1
fi
if [[ "$filesystem" != "xfs" ]]; then
    printf 'fast storage requires XFS; found %s\n' "$filesystem" >&2
    exit 1
fi

probe_source="$(mktemp "$PREPARED_DIR/.soma-reflink-source.XXXXXX")"
probe_directory="$(mktemp -d "$HEAD_DIR/.soma-reflink-probe.XXXXXX")"
probe_head="$probe_directory/head"
cleanup() { rm -f -- "$probe_source" "$probe_head"; rmdir "$probe_directory"; }
trap cleanup EXIT
truncate -s 1048576 "$probe_source"
cp --reflink=always -- "$probe_source" "$probe_head"
cmp -- "$probe_source" "$probe_head"
printf 'fast storage ready: device=%s filesystem=%s reflink=proven\n' \
    "$head_device" "$filesystem"
