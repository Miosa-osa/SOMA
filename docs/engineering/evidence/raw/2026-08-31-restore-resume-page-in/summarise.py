"""Summarise one cohort: receipt segments, and page-in state where it was read."""

import glob
import json
import os
import statistics
import sys

directory, label, mem, conc, mode = sys.argv[1:6]

stages, totals = {}, []
for path in glob.glob(os.path.join(directory, "*.json")):
    try:
        record = json.loads(open(path).read().strip().splitlines()[-1])
        marks = {m["kind"]: m["elapsed_ns"] / 1e6 for m in record["receipt"]["milestones"]}
    except Exception:
        continue
    if "command_finished" not in marks:
        continue
    totals.append(marks["command_finished"])
    for kind, value in marks.items():
        stages.setdefault(kind, []).append(value)

order = ["admitted", "machine_launched", "ready", "command_started", "command_finished"]
median = {k: round(statistics.median(v), 2) for k, v in stages.items()}
previous, delta = 0.0, {}
for kind in order:
    if kind in median:
        delta[kind] = round(median[kind] - previous, 2)
        previous = median[kind]

pagein = [json.loads(open(p).read()) for p in glob.glob(os.path.join(directory, "*.smaps"))
          if os.path.getsize(p)]
pagein = [p for p in pagein if p.get("rss_kb")]


def median_of(key):
    values = [p[key] for p in pagein if p.get(key) is not None]
    return round(statistics.median(values), 1) if values else None


print(json.dumps({
    "label": label, "memory_mib": int(mem), "concurrency": int(conc), "mode": mode,
    "succeeded": len(totals),
    "tti_p50_ms": round(statistics.median(totals), 2) if totals else None,
    "stage_delta_ms": delta,
    "pagein_samples": len(pagein),
    "faulted_pages_p50": median_of("faulted_pages"),
    "cow_pages_p50": median_of("cow_pages"),
    "rss_kb_p50": median_of("rss_kb"),
    "process_minor_faults_p50": median_of("process_minor_faults"),
}))
