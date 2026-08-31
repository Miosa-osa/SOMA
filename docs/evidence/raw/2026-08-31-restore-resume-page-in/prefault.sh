#!/usr/bin/env bash
# Both arms of the pre-fault experiment, from one binary, interleaved.
#
# The two arms differ only in whether the restored memory image is walked once before the vCPU
# is armed. Interleaving them means a drift in host load cannot masquerade as a result, and the
# minor-fault count is taken beside the timing so the two can be read together.
set -uo pipefail
STORE="$1"; MEM="$2"; DISK="$3"; IMAGE="$4"; SAMPLES="$5"; LABEL="$6"
shift 6
PROG=("$@")
REPO=/srv/soma/pagein/SOMA
cd "$REPO"
export SOMA_GENERATION_STORE="$STORE" SOMA_HEAD_DIR=/srv/soma/pagein/heads
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
mkdir -p /srv/soma/pagein/heads

for arm in cold warm; do
    rm -rf "/srv/soma/pagein/tl-$LABEL-$arm"
    mkdir -p "/srv/soma/pagein/tl-$LABEL-$arm"
done
unset SOMA_KVM_PREFAULT_MEMORY
./target/release/soma --format json --backend kvm run --memory-mib "$MEM" \
    --storage-mib "$DISK" "$IMAGE" -- "${PROG[@]}" >/dev/null 2>&1

for _ in $(seq 1 "$SAMPLES"); do
    for arm in cold warm; do
        if [[ "$arm" == warm ]]; then export SOMA_KVM_PREFAULT_MEMORY=1
        else unset SOMA_KVM_PREFAULT_MEMORY; fi
        SOMA_KVM_TIMELINE="/srv/soma/pagein/tl-$LABEL-$arm" \
        ./target/release/soma --format json --backend kvm run --memory-mib "$MEM" \
            --storage-mib "$DISK" "$IMAGE" -- "${PROG[@]}" >/dev/null 2>&1
    done
done

for arm in cold warm; do
    python3 /srv/soma/pagein/resume.py "/srv/soma/pagein/tl-$LABEL-$arm" "$LABEL-$arm"
done
