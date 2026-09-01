#!/usr/bin/env bash
# Per-stage medians for one cohort of one Generation, retained to one file.
#
# The same harness the serialization record used, with the head root, the head shard count, and
# the template fan taken from the environment instead of fixed, so one arm differs from another
# only by those three variables.
#
# Same shape as the burst harness: one barrier releases every slot together, receipts are parsed
# only after the last sandbox exits so no interpreter competes with the measurement for a core.
set -uo pipefail
STORE="$1"; MEM="$2"; DISK="$3"; CONC="$4"; OUT="$5"
cd "${SOMA_REPO:-/srv/soma/hc/repo}"
export SOMA_GENERATION_STORE="$STORE"
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
RAW="$(mktemp -d)"; BAR="$RAW/barrier"
mkdir -p "$(dirname "$OUT")"
one() {
    while [[ ! -f "$BAR" ]]; do :; done
    ./target/release/soma --format json --backend kvm run busybox:stable-musl \
        --memory-mib "$MEM" --storage-mib "$DISK" \
        -- /bin/echo soma-ok > "$RAW/$1.json" 2>/dev/null
}
for i in $(seq 1 "$CONC"); do one "$i" & done
sleep 1
: > "$BAR"
wait
python3 - "$RAW" "$OUT" "$CONC" <<PYEOF
import base64, glob, json, os, statistics, sys
raw, out, conc = sys.argv[1], sys.argv[2], int(sys.argv[3])
rows, stages = [], {}
for path in glob.glob(os.path.join(raw, "*.json")):
    rec = {"ok": 0, "tti_ms": None}
    try:
        r = json.loads(open(path).read().strip().splitlines()[-1])
        stdout = base64.b64decode(r["result"]["stdout"]["data"]).decode() if r.get("result") else ""
        ms = {m["kind"]: m["elapsed_ns"] / 1e6 for m in r["receipt"]["milestones"]}
        if "soma-ok" in stdout and "command_finished" in ms:
            rec = {"ok": 1, "tti_ms": round(ms["command_finished"], 1),
                   "stages": {k: round(v, 1) for k, v in ms.items()}}
            for k, v in ms.items():
                stages.setdefault(k, []).append(v)
    except Exception:
        pass
    rows.append(rec)
open(out, "w").write("".join(json.dumps(r) + "\n" for r in rows))
ok = sorted(r["tti_ms"] for r in rows if r["ok"])
def pct(v, p): return v[max(1, -(-p * len(v) // 100)) - 1] if v else None
order = ["accepted", "workload_resolved", "admitted", "machine_launched",
         "ready", "command_started", "command_finished"]
med = {k: round(statistics.median(stages[k]), 1) for k in order if k in stages}
prev, deltas = 0.0, {}
for k in order:
    if k in med:
        deltas[k] = round(med[k] - prev, 1); prev = med[k]
launch = sorted(stages.get("machine_launched", [])) 
adm = sorted(stages.get("admitted", []))
seg = sorted(b - a for a, b in zip(stages.get("admitted", []), stages.get("machine_launched", [])))
def s(v, p): return round(v[max(1, -(-p * len(v) // 100)) - 1], 2) if v else None
summary = {"store": os.path.basename(os.path.dirname(out)) if False else None,
           "concurrency": conc, "attempted": len(rows), "succeeded": len(ok),
           "success_rate": round(len(ok) / len(rows), 4) if rows else 0,
           "tti_p50_ms": pct(ok, 50), "tti_p95_ms": pct(ok, 95), "tti_p99_ms": pct(ok, 99),
           "stage_median_ms": med, "stage_delta_ms": deltas,
           "admitted_to_machine_launched_ms": {
               "min": s(seg, 1), "p50": s(seg, 50), "p95": s(seg, 95), "p99": s(seg, 99),
               "max": round(seg[-1], 2) if seg else None}}
open(out + ".summary", "w").write(json.dumps(summary) + "\n")
print(json.dumps(summary, indent=2))
PYEOF
rm -rf "$RAW"
