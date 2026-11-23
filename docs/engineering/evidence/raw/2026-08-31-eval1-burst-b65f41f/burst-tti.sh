#!/usr/bin/env bash
# Burst TTI and per-stage medians from one cohort, retained to one file.
#
# The stage medians and the TTI were previously produced by two different scripts on two
# different runs, and the stage script only printed its result. Both now come from the same
# cohort and are written down, so the breakdown and the total describe the same sandboxes.
#
# Boundary: ComputeSDK stops at the first successful command, so the receipt's `command_finished`
# milestone is the TTI. Destroy happens and is never counted. Receipts are parsed only after the
# last sandbox exits, so no interpreter competes with the measurement for a core.
set -uo pipefail
CONC="$1"; OUT="$2"
cd /srv/soma/SOMA
export SOMA_GENERATION_STORE=/srv/soma/prepared
export SOMA_HEAD_DIR=/srv/soma/heads
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
RAW="$(mktemp -d)"; BAR="$RAW/barrier"
mkdir -p "$(dirname "$OUT")"
one() {
    while [[ ! -f "$BAR" ]]; do :; done      # one barrier releases every slot together
    ./target/release/soma --format json --backend kvm run node:22 \
        -- /usr/local/bin/node --version > "$RAW/$1.json" 2>/dev/null
}
for i in $(seq 1 "$CONC"); do one "$i" & done
sleep 1
: > "$BAR"
wait
python3 - "$RAW" "$OUT" "$CONC" <<'PYEOF'
import base64, glob, json, os, statistics, sys
raw, out, conc = sys.argv[1], sys.argv[2], int(sys.argv[3])
rows, stages = [], {}
for path in glob.glob(os.path.join(raw, "*.json")):
    rec = {"ok": 0, "tti_ms": None}
    try:
        r = json.loads(open(path).read().strip().splitlines()[-1])
        stdout = base64.b64decode(r["result"]["stdout"]["data"]).decode() if r.get("result") else ""
        ms = {m["kind"]: m["elapsed_ns"] / 1e6 for m in r["receipt"]["milestones"]}
        if "v22" in stdout and "command_finished" in ms:
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
summary = {"concurrency": conc, "attempted": len(rows), "succeeded": len(ok),
           "success_rate": round(len(ok) / len(rows), 4) if rows else 0,
           "tti_p50_ms": pct(ok, 50), "tti_p95_ms": pct(ok, 95), "tti_p99_ms": pct(ok, 99),
           "tti_min_ms": ok[0] if ok else None, "tti_max_ms": ok[-1] if ok else None,
           "stage_median_ms": med, "stage_delta_ms": deltas}
open(out + ".summary", "w").write(json.dumps(summary) + "\n")
print(json.dumps(summary, indent=2))
PYEOF
rm -rf "$RAW"
