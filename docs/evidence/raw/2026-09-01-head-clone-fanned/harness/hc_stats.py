"""Cohort and launch distributions per arm, discarding cohorts taken on a busy host.

The retained serialization record's `cohort_stats.py` with three arms instead of two, and with
the bimodal split reported explicitly: the earlier record found thirty of forty writable cohorts
in one mode and ten in another, so a single median hides the thing being fixed.

Usage: hc_stats.py <raw directory> [busy limit] [arm,arm,...]
"""

import glob
import json
import os
import statistics
import sys

root = sys.argv[1]
LIMIT = float(sys.argv[2]) if len(sys.argv) > 2 else 12.0
ARMS = sys.argv[3].split(",") if len(sys.argv) > 3 else ["base", "shard", "fan"]

loads = {}
if os.path.exists(f"{root}/load.jsonl"):
    for line in open(f"{root}/load.jsonl"):
        record = json.loads(line)
        loads[(record["arm"], record["cohort"])] = record


def pct(values, point):
    ordered = sorted(values)
    return round(ordered[max(1, -(-point * len(ordered) // 100)) - 1], 2)


for arm in ARMS:
    cohort_tti, cohort_seg, seg_all, tti_all, ok, discarded = [], [], [], [], 0, 0
    busiest = 0.0
    for path in sorted(glob.glob(f"{root}/{arm}-*.jsonl.summary")):
        index = int(os.path.basename(path).split("-")[1].split(".")[0])
        record = loads.get((arm, index))
        if record and max(record["busy_before"], record["busy_after"]) > LIMIT:
            discarded += 1
            continue
        if record:
            busiest = max(busiest, record["busy_before"], record["busy_after"])
        summary = json.loads(open(path).read())
        if summary["succeeded"] != summary["attempted"]:
            discarded += 1
            continue
        cohort_tti.append(summary["tti_p50_ms"])
        cohort_seg.append(summary["admitted_to_machine_launched_ms"]["p50"])
        ok += summary["succeeded"]
        for line in open(path[: -len(".summary")]):
            launch = json.loads(line)
            if launch["ok"]:
                tti_all.append(launch["tti_ms"])
                seg_all.append(launch["stages"]["machine_launched"] - launch["stages"]["admitted"])
    if not cohort_tti:
        continue
    ordered = sorted(cohort_tti)
    fast = [value for value in ordered if value < 2 * ordered[0]]
    print(
        json.dumps(
            {
                "arm": arm,
                "cohorts_kept": len(cohort_tti),
                "cohorts_discarded": discarded,
                "launches": ok,
                "busiest_percent": round(busiest, 2),
                "cohort_p50_min": min(cohort_tti),
                "cohort_p50_max": max(cohort_tti),
                "cohort_p50_spread": round(max(cohort_tti) / min(cohort_tti), 2),
                "cohort_p50_median": round(statistics.median(cohort_tti), 1),
                "cohorts_within_2x_of_fastest": len(fast),
                "clone_seg_cohort_p50_min": min(cohort_seg),
                "clone_seg_cohort_p50_max": max(cohort_seg),
                "clone_seg_cohort_p50_median": round(statistics.median(cohort_seg), 2),
                "tti_p50": pct(tti_all, 50),
                "tti_p95": pct(tti_all, 95),
                "tti_p99": pct(tti_all, 99),
                "tti_max": round(max(tti_all), 2),
                "clone_seg_p50": pct(seg_all, 50),
                "clone_seg_p95": pct(seg_all, 95),
                "clone_seg_p99": pct(seg_all, 99),
                "clone_seg_max": round(max(seg_all), 2),
                "clone_seg_per_launch_us": round(1000 * statistics.mean(seg_all), 1),
            }
        )
    )
    print("  cohort medians:", ordered)
