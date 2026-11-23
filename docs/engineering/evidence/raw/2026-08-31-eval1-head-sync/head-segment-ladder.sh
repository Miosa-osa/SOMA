#!/usr/bin/env bash
# Percentiles of the receipt's `admitted` to `machine_launched` segment for one cohort.
#
# That segment is dominated by the private overlay head the Instance is given, so it is the
# measurement behind `results.md`. It reads only the receipt, so it needs no instrumented build:
# point it at a prepared Generation and pass the memory and storage the Generation was captured
# with, because a Generation restores at no other shape.
#
# Usage: head-segment-ladder.sh STORE IMAGE MEMORY_MIB STORAGE_MIB CONCURRENCY REPS COMMAND...
set -uo pipefail
STORE="$1"; IMAGE="$2"; MEM="$3"; STO="$4"; CONC="$5"; REPS="$6"; shift 6
export SOMA_GENERATION_STORE="$STORE"
RAW="$(mktemp -d)"; BAR="$RAW/barrier"
one() {
    while [[ ! -f "$BAR" ]]; do :; done      # one barrier releases every slot together
    soma --format json --backend kvm \
        run --memory-mib "$MEM" --storage-mib "$STO" "$IMAGE" \
        -- "$@" > "$RAW/$SLOT.json" 2>/dev/null
}
for rep in $(seq 1 "$REPS"); do
    rm -f "$BAR"
    for i in $(seq 1 "$CONC"); do SLOT="$rep-$i" one "$@" & done
    sleep 1
    : > "$BAR"
    wait
done
python3 - "$RAW" "$CONC" "$REPS" <<'PYEOF'
import glob, json, os, sys
raw, conc, reps = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
segment, tti = [], []
for path in glob.glob(os.path.join(raw, "*.json")):
    try:
        receipt = json.loads(open(path).read().strip().splitlines()[-1])["receipt"]
        stamps = {m["kind"]: m["elapsed_ns"] / 1e6 for m in receipt["milestones"]}
    except Exception:
        continue
    if "machine_launched" in stamps and "admitted" in stamps:
        segment.append(stamps["machine_launched"] - stamps["admitted"])
    if "command_finished" in stamps:
        tti.append(stamps["command_finished"])
segment.sort()
tti.sort()
def pct(values, p):
    return round(values[max(1, -(-p * len(values) // 100)) - 1], 1) if values else None
print(json.dumps({
    "concurrency": conc, "reps": reps, "launched": len(segment), "completed": len(tti),
    "segment_p50_ms": pct(segment, 50), "segment_p95_ms": pct(segment, 95),
    "segment_max_ms": round(segment[-1], 1) if segment else None,
    "tti_p50_ms": pct(tti, 50), "tti_p95_ms": pct(tti, 95),
}))
PYEOF
rm -rf "$RAW"
