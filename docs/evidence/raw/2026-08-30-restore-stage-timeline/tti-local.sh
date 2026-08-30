#!/usr/bin/env bash
# Burst TTI on the ComputeSDK boundary, run against a local prepared store.
#
# Same boundary as the eval-1 harness: the timer stops at the receipt's `command_finished`
# milestone, which is where ComputeSDK stops. Destroy happens but is never counted. Every
# receipt is parsed only after the last sandbox has exited, so no interpreter competes with
# the measurement for a core.
set -uo pipefail
CONC="$1"; OUT="$2"
REPO="$HOME/projects/SOMA-wt/audit2"
S=/tmp/claude-1000/-home-miosa/2cb96714-1a75-410b-8943-4396b1891a64/scratchpad
RAW="$(mktemp -d)"
cd "$REPO"
export SOMA_GENERATION_STORE="$S/store2"
export SOMA_HEAD_DIR="$S/heads"
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
mkdir -p "$SOMA_HEAD_DIR" "$(dirname "$OUT")"
BAR="$RAW/barrier"
one() {
    while [[ ! -f "$BAR" ]]; do :; done      # one barrier releases every slot together
    ./target/release/soma --format json --backend kvm run node:22 \
        -- /usr/local/bin/node --version > "$RAW/$1.json" 2>/dev/null
}
for i in $(seq 1 "$CONC"); do one "$i" & done
sleep 1
: > "$BAR"
wait
python3 - "$RAW" "$OUT" "$CONC" <<'PY'
import base64, glob, json, os, sys
raw, out, conc = sys.argv[1], sys.argv[2], int(sys.argv[3])
rows = []
for path in glob.glob(os.path.join(raw, "*.json")):
    rec = {"ok": 0, "tti_ms": None, "stages": None}
    try:
        r = json.loads(open(path).read().strip().splitlines()[-1])
        if r.get("status") == "ok":
            stdout = base64.b64decode(r["result"]["stdout"]["data"]).decode()
            ms = {m["kind"]: m["elapsed_ns"] / 1e6 for m in r["receipt"]["milestones"]}
            if "v22" in stdout and "command_finished" in ms:
                rec = {"ok": 1, "tti_ms": round(ms["command_finished"], 1),
                       "stages": {k: round(v, 1) for k, v in ms.items()}}
    except Exception:
        pass
    rows.append(rec)
open(out, "w").write("".join(json.dumps(r) + "\n" for r in rows))
ok = sorted(r["tti_ms"] for r in rows if r["ok"])
def pct(v, p): return v[max(1, -(-p * len(v) // 100)) - 1] if v else None
def stage(name):
    vals = sorted(r["stages"][name] for r in rows if r["ok"] and name in r["stages"])
    return pct(vals, 50)
print(json.dumps({
    "concurrency": conc, "attempted": len(rows), "succeeded": len(ok),
    "success_rate": round(len(ok) / len(rows), 4) if rows else 0,
    "tti_p50_ms": pct(ok, 50), "tti_p95_ms": pct(ok, 95), "tti_p99_ms": pct(ok, 99),
    "tti_min_ms": ok[0] if ok else None, "tti_max_ms": ok[-1] if ok else None,
    "stage_p50": {k: stage(k) for k in
                  ("admitted", "machine_launched", "ready", "command_started", "command_finished", "cleanup_finished")},
}, indent=2))
PY
rm -rf "$RAW"
