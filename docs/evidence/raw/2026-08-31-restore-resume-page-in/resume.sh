#!/usr/bin/env bash
# The resume window, one sandbox at a time, from the machine's own milestones.
#
# `SOMA_KVM_TIMELINE` now carries the two sides of the first `KVM_RUN`, so the block between
# arming the vCPU and the guest erasing the launch page splits three ways without a profiler
# and without root: arming to entry, the first call itself, and everything after it.
set -uo pipefail
STORE="$1"; MEM="$2"; DISK="$3"; IMAGE="$4"; SAMPLES="$5"; LABEL="$6"
shift 6
REPO=/srv/soma/pagein/SOMA
DIR=/srv/soma/pagein/tl-$LABEL
rm -rf "$DIR"; mkdir -p "$DIR" /srv/soma/pagein/heads
cd "$REPO"
export SOMA_GENERATION_STORE="$STORE" SOMA_HEAD_DIR=/srv/soma/pagein/heads
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1 SOMA_KVM_TIMELINE="$DIR"

./target/release/soma --format json --backend kvm run --memory-mib "$MEM" \
    --storage-mib "$DISK" "$IMAGE" -- "$@" >/dev/null 2>&1
for _ in $(seq 1 "$SAMPLES"); do
    ./target/release/soma --format json --backend kvm run --memory-mib "$MEM" \
        --storage-mib "$DISK" "$IMAGE" -- "$@" >/dev/null 2>&1
done
python3 /srv/soma/pagein/resume.py "$DIR" "$LABEL"
