"""Cohort and launch distributions per arm, discarding cohorts taken on a busy host."""

import glob
import json
import os
import statistics
import sys

LIMIT = float(sys.argv[2]) if len(sys.argv) > 2 else 12.0
root = sys.argv[1]
loads = {}
if os.path.exists(f"{root}/load.jsonl"):
    for line in open(f"{root}/load.jsonl"):
        record = json.loads(line)
        loads[(record["arm"], record["cohort"])] = record


def pct(values, point):
    ordered = sorted(values)
    return round(ordered[max(1, -(-point * len(ordered) // 100)) - 1], 2)


for arm in ("rw", "ro"):
    cohort_tti, cohort_seg, seg_all, tti_all, ok, discarded = [], [], [], [], 0, 0
    for path in sorted(glob.glob(f"{root}/{arm}-*.jsonl.summary")):
        index = int(os.path.basename(path).split("-")[1].split(".")[0])
        record = loads.get((arm, index))
        if record and max(record["busy_before"], record["busy_after"]) > LIMIT:
            discarded += 1
            continue
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
    print(
        json.dumps(
            {
                "arm": arm,
                "cohorts_kept": len(cohort_tti),
                "cohorts_discarded": discarded,
                "launches": ok,
                "cohort_p50_min": min(cohort_tti),
                "cohort_p50_max": max(cohort_tti),
                "cohort_p50_spread": round(max(cohort_tti) / min(cohort_tti), 2),
                "cohort_p50_median": round(statistics.median(cohort_tti), 1),
                "clone_seg_cohort_p50_min": min(cohort_seg),
                "clone_seg_cohort_p50_max": max(cohort_seg),
                "tti_p50": pct(tti_all, 50),
                "tti_p95": pct(tti_all, 95),
                "tti_p99": pct(tti_all, 99),
                "tti_max": round(max(tti_all), 2),
                "clone_seg_p50": pct(seg_all, 50),
                "clone_seg_p95": pct(seg_all, 95),
                "clone_seg_p99": pct(seg_all, 99),
                "clone_seg_max": round(max(seg_all), 2),
            }
        )
    )
    print("  cohort medians:", sorted(cohort_tti))
