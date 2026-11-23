#!/usr/bin/env bash
# Twenty sequential sandboxes of one Generation, one at a time, retained as one line each.
#
# Concurrency 1 is measured by repetition rather than by one sample: the segment being compared
# is a few milliseconds wide and one run of it says nothing about its spread.
set -uo pipefail
STORE="$1"; MEM="$2"; DISK="$3"; RUNS="$4"; OUT="$5"
cd /srv/soma/SOMA-devset
export SOMA_GENERATION_STORE="$STORE"
export SOMA_HEAD_DIR=/srv/soma/heads
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
mkdir -p "$(dirname "$OUT")"
: > "$OUT"
for i in $(seq 1 "$RUNS"); do
    ./target/release/soma --format json --backend kvm run busybox:stable-musl \
        --memory-mib "$MEM" --storage-mib "$DISK" \
        -- /bin/echo soma-ok 2>/dev/null | tail -1 >> "$OUT"
done
python3 - "$OUT" <<PYEOF
import base64, json, statistics, sys
rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
seg, tti, ok = [], [], 0
for r in rows:
    try:
        out = base64.b64decode(r["result"]["stdout"]["data"]).decode()
        ms = {m["kind"]: m["elapsed_ns"] / 1e6 for m in r["receipt"]["milestones"]}
        if "soma-ok" in out:
            ok += 1
            seg.append(ms["machine_launched"] - ms["admitted"])
            tti.append(ms["command_finished"])
    except Exception:
        pass
seg.sort(); tti.sort()
def p(v, q): return round(v[max(1, -(-q * len(v) // 100)) - 1], 2) if v else None
print(json.dumps({"runs": len(rows), "succeeded": ok,
                  "admitted_to_machine_launched_ms": {
                      "min": p(seg, 1), "p50": p(seg, 50), "p95": p(seg, 95),
                      "max": round(seg[-1], 2) if seg else None,
                      "mean": round(statistics.mean(seg), 2) if seg else None},
                  "tti_ms": {"p50": p(tti, 50), "p95": p(tti, 95)}}, indent=2))
PYEOF
