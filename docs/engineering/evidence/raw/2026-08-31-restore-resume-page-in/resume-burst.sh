#!/usr/bin/env bash
# The resume window across one cohort released from a single barrier.
#
# The same milestones as the sequential run, taken while a hundred sandboxes compete, so the
# split between the first `KVM_RUN` call and everything after it can be read under load as
# well as alone.
set -uo pipefail
STORE="$1"; MEM="$2"; DISK="$3"; IMAGE="$4"; CONC="$5"; LABEL="$6"
shift 6
PROG=("$@")
REPO=/srv/soma/pagein/SOMA
DIR=/srv/soma/pagein/tl-$LABEL
rm -rf "$DIR"; mkdir -p "$DIR" /srv/soma/pagein/heads
cd "$REPO"
export SOMA_GENERATION_STORE="$STORE" SOMA_HEAD_DIR=/srv/soma/pagein/heads
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1

./target/release/soma --format json --backend kvm run --memory-mib "$MEM" \
    --storage-mib "$DISK" "$IMAGE" -- "${PROG[@]}" >/dev/null 2>&1
export SOMA_KVM_TIMELINE="$DIR"
BAR="$DIR/.barrier"
one() {
    while [[ ! -f "$BAR" ]]; do :; done
    ./target/release/soma --format json --backend kvm run --memory-mib "$MEM" \
        --storage-mib "$DISK" "$IMAGE" -- "${PROG[@]}" >/dev/null 2>&1
}
for _ in $(seq 1 "$CONC"); do one & done
sleep 1
: > "$BAR"
wait
python3 /srv/soma/pagein/resume.py "$DIR" "$LABEL"
