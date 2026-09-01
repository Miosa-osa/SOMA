#!/usr/bin/env bash
# One restored launch with kvm page-fault, entry and exit tracepoints recorded around it.
set -uo pipefail
STORE="${1:?store}"; MEM="${2:?mem}"; DISK="${3:?disk}"; IMAGE="${4:?image}"; LABEL="${5:?label}"
shift 5
REPO=/srv/soma/pagein/SOMA
OUT=/srv/soma/ept/raw
mkdir -p "$OUT" /srv/soma/ept/heads
cd "$REPO"
export SOMA_GENERATION_STORE="$STORE" SOMA_HEAD_DIR=/srv/soma/ept/heads
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1

# Host idleness beside the sample, from /proc/stat rather than the load average.
head -1 /proc/stat > "$OUT/$LABEL.stat-before"
sudo perf record -a -q -e kvm:kvm_page_fault,kvm:kvm_entry,kvm:kvm_exit,kvm:kvm_try_async_get_page,kvm:kvm_async_pf_not_present,kvm:kvm_async_pf_repeated_fault,kvmmmu:fast_page_fault,kvmmmu:kvm_tdp_mmu_spte_changed \
    -o "$OUT/$LABEL.data" -- sleep 3 >/dev/null 2>&1 &
PERF=$!
sleep 1
./target/release/soma --format json --backend kvm run --memory-mib "$MEM" \
    --storage-mib "$DISK" "$IMAGE" -- "$@" > "$OUT/$LABEL.receipt.json" 2>"$OUT/$LABEL.err"
wait $PERF
head -1 /proc/stat > "$OUT/$LABEL.stat-after"
sudo chown "$(id -u)" "$OUT/$LABEL.data"
perf script -i "$OUT/$LABEL.data" > "$OUT/$LABEL.txt" 2>/dev/null
python3 /srv/soma/ept/seg.py < "$OUT/$LABEL.receipt.json"
printf "page faults recorded: %s\n" "$(grep -c kvm_page_fault "$OUT/$LABEL.txt")"
