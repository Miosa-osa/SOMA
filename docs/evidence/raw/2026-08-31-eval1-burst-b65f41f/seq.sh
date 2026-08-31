#!/usr/bin/env bash
# Sequential TTI on the ComputeSDK boundary: one sandbox at a time, N samples.
#
# The retained sequential result before this had one sample, which is not a distribution and
# cannot carry a p50. This runs N of them back to back, with nothing else on the host, and writes
# every sample so the summary can be recomputed from the raw file.
set -uo pipefail
N="${1:-25}"; OUT="$2"
cd /srv/soma/SOMA
export SOMA_GENERATION_STORE=/srv/soma/prepared
export SOMA_HEAD_DIR=/srv/soma/heads
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1
RAW="$(mktemp -d)"
for i in $(seq 1 "$N"); do
    ./target/release/soma --format json --backend kvm run node:22 \
        -- /usr/local/bin/node --version > "$RAW/$i.json" 2>/dev/null
done
python3 - "$RAW" "$OUT" "$N" <<'PY'
import base64, glob, json, os, sys
raw, out, n = sys.argv[1], sys.argv[2], int(sys.argv[3])
rows = []
for path in sorted(glob.glob(os.path.join(raw, "*.json"))):
    rec = {"ok": 0, "tti_ms": None}
    try:
        r = json.loads(open(path).read().strip().splitlines()[-1])
        stdout = base64.b64decode(r["result"]["stdout"]["data"]).decode() if r.get("result") else ""
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
print(json.dumps({"mode": "sequential", "samples": n, "succeeded": len(ok),
                  "tti_p50_ms": pct(ok, 50), "tti_p95_ms": pct(ok, 95),
                  "tti_min_ms": ok[0] if ok else None, "tti_max_ms": ok[-1] if ok else None}))
PY
rm -rf "$RAW"
