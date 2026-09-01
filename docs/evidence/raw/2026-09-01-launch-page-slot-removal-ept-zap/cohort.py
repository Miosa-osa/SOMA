"""Reduces one cohort of receipts to its TTI and ready-segment medians.

Host idleness is taken from /proc/stat across the cohort rather than from the load average,
because a hundred-way cohort raises the load average by itself and other work on the host is
exactly what has to be told apart from it.
"""
import base64, glob, json, os, statistics, sys

raw, out, conc, label = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]


def stat(path):
    return [int(x) for x in open(path).read().split()[1:11]]


before, after = stat(os.path.join(raw, "stat-before")), stat(os.path.join(raw, "stat-after"))
delta = [a - b for a, b in zip(after, before)]
busy = 100 * (1 - delta[3] / max(sum(delta), 1))

tti, ready, segment, ok = [], [], [], 0
for path in glob.glob(os.path.join(raw, "*.json")):
    try:
        r = json.loads(open(path).read().strip().splitlines()[-1])
        stdout = base64.b64decode(r["result"]["stdout"]["data"]).decode() if r.get("result") else ""
        ms = {m["kind"]: m["elapsed_ns"] / 1e6 for m in r["receipt"]["milestones"]}
    except (ValueError, KeyError, TypeError):
        continue
    if "v22" not in stdout or "command_finished" not in ms:
        continue
    ok += 1
    tti.append(ms["command_finished"])
    ready.append(ms["ready"])
    segment.append(ms["ready"] - ms["machine_launched"])


def median(values):
    return round(statistics.median(values), 2) if values else None


result = {
    "label": label, "concurrency": conc, "launched": conc, "succeeded": ok,
    "host_busy_percent_during_cohort": round(busy, 1),
    "tti_ms": median(tti), "ready_ms": median(ready),
    "machine_launched_to_ready_ms": median(segment),
    "tti_samples": sorted(round(v, 2) for v in tti),
}
open(out, "w").write(json.dumps(result, indent=1) + "\n")
print(f"{label:12s} conc {conc:3d} ok {ok:3d}/{conc:3d}  tti {result["tti_ms"]}  "
      f"ready {result["ready_ms"]}  segment {result["machine_launched_to_ready_ms"]}  "
      f"host busy {result["host_busy_percent_during_cohort"]}%")
