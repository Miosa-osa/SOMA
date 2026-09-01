#!/usr/bin/env bash
# One restored launch, with the guest s EPT violations traced around it.
#
# The window is the machine s own RunStart to Ready, read from the timeline this launch wrote,
# so it does not depend on the recording overhead changing how long the resume takes.
set -uo pipefail
STORE="${1:?store}"; MEM="${2:?mem}"; DISK="${3:?disk}"; IMAGE="${4:?image}"; LABEL="${5:?label}"
shift 5
REPO=/srv/soma/pagein/SOMA
OUT=/srv/soma/ept/raw
EVENTS="${SOMA_EPT_EVENTS:-kvm:kvm_page_fault,kvm:kvm_entry,kvm:kvm_exit}"
mkdir -p "$OUT" /srv/soma/ept/heads "$OUT/$LABEL.tl"
rm -f "$OUT/$LABEL.tl"/*
cd "$REPO"
export SOMA_GENERATION_STORE="$STORE" SOMA_HEAD_DIR=/srv/soma/ept/heads
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1 SOMA_KVM_TIMELINE="$OUT/$LABEL.tl"

head -1 /proc/stat > "$OUT/$LABEL.stat-before"
sudo perf record -a -q -e "$EVENTS" -o "$OUT/$LABEL.data" -- sleep 3 >/dev/null 2>&1 &
PERF=$!
sleep 1
./target/release/soma --format json --backend kvm run --memory-mib "$MEM" \
    --storage-mib "$DISK" "$IMAGE" -- "$@" > "$OUT/$LABEL.receipt.json" 2>"$OUT/$LABEL.err"
wait $PERF
head -1 /proc/stat > "$OUT/$LABEL.stat-after"
sudo chown "$(id -u)" "$OUT/$LABEL.data"
perf script -i "$OUT/$LABEL.data" > "$OUT/$LABEL.txt" 2>/dev/null
