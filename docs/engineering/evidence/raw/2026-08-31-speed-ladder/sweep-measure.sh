#!/usr/bin/env bash
# Measures every prepared configuration across a concurrency ladder.
#
# The point is not one number. It is which dimension actually moves the result: the workload's
# own weight, the memory the machine restores, or the number of sandboxes competing for the host.
# Each cohort writes its samples so a figure can be recomputed rather than believed, and every
# cohort names the configuration it measured.
set -uo pipefail
REPO=/srv/soma/SOMA
OUT=/srv/soma/bench/sweep-results.jsonl
: > "$OUT"
cd "$REPO"
export SOMA_HEAD_DIR=/srv/soma/heads SOMA_ALLOW_UNCERTIFIED_GENERATION=1

# A machine restores exactly the memory its snapshot was captured with, so the shape is not a
# flag a measurement may leave at its default: asking for the wrong size is refused before the
# machine exists. Each row therefore carries the shape its Generation was built for.
# name  image  mem  disk  probe-program  probe-arg  expected
CASES=(
  "bb-128-1024     busybox:stable-musl  128   1024   /bin/busybox  --help  BusyBox"
  "bb-512-10240    busybox:stable-musl  512   10240  /bin/busybox  --help  BusyBox"
  "bb-1024-10240   busybox:stable-musl  1024  10240  /bin/busybox  --help  BusyBox"
  "node-1024-10240 node:22              1024  10240  /usr/local/bin/node  --version  v22"
)
LADDER=(1 10 25 50 100)

for row in "${CASES[@]}"; do
    read -r name image mem disk prog arg expect <<< "$row"
    store="/srv/soma/sweep/$name"
    [[ -f "$store/.ready" ]] || { echo "skip $name (not prepared)"; continue; }
    export SOMA_GENERATION_STORE="$store"

    # One warming launch so every cohort measures a warm page cache rather than the first touch.
    ./target/release/soma --format json --backend kvm run --memory-mib "$mem" --storage-mib "$disk" "$image" -- "$prog" "$arg" >/dev/null 2>&1

    for conc in "${LADDER[@]}"; do
        raw=$(mktemp -d); bar="$raw/barrier"
        one() { while [[ ! -f "$bar" ]]; do :; done
            ./target/release/soma --format json --backend kvm run --memory-mib "$mem" --storage-mib "$disk" "$image" -- "$prog" "$arg" > "$raw/$1.json" 2>/dev/null; }
        for i in $(seq 1 "$conc"); do one "$i" & done
        sleep 1; : > "$bar"; wait
        python3 - "$raw" "$name" "$image" "$mem" "$conc" "$expect" "$OUT" <<'PY'
import base64, glob, json, os, statistics, sys
raw, name, image, mem, conc, expect, out = sys.argv[1:8]
rows, stages = [], {}
for path in glob.glob(os.path.join(raw, "*.json")):
    try:
        r = json.loads(open(path).read().strip().splitlines()[-1])
        text = base64.b64decode(r["result"]["stdout"]["data"]).decode(errors="replace") if r.get("result") else ""
        ms = {m["kind"]: m["elapsed_ns"] / 1e6 for m in r["receipt"]["milestones"]}
        ok = expect in text and "command_finished" in ms
        rows.append(ms["command_finished"] if ok else None)
        if ok:
            for k, v in ms.items():
                stages.setdefault(k, []).append(v)
    except Exception:
        rows.append(None)
good = sorted(v for v in rows if v is not None)
def pct(v, p): return round(v[max(1, -(-p * len(v) // 100)) - 1], 1) if v else None
med = {k: round(statistics.median(v), 1) for k, v in stages.items()}
order = ["admitted", "machine_launched", "ready", "command_finished"]
prev, delta = 0.0, {}
for k in order:
    if k in med:
        delta[k] = round(med[k] - prev, 1); prev = med[k]
open(out, "a").write(json.dumps({
    "config": name, "image": image, "memory_mib": int(mem), "concurrency": int(conc),
    "attempted": len(rows), "succeeded": len(good),
    "tti_p50_ms": pct(good, 50), "tti_p95_ms": pct(good, 95),
    "tti_min_ms": good[0] if good else None, "tti_max_ms": good[-1] if good else None,
    "stage_delta_ms": delta,
}) + "\n")
print(f"{name:<18}c={conc:<4}ok={len(good)}/{len(rows)}  p50={pct(good,50)}ms  min={good[0] if good else None}")
PY
        rm -rf "$raw"
    done
done
echo "=== sweep measure complete ==="
