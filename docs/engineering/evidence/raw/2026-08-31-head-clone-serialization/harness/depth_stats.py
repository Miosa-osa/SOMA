"""Clone segment cost and cohort to cohort spread against the concurrency of the cohort."""

import glob
import json
import os
import statistics
import sys

root = sys.argv[1]
LIMIT = float(sys.argv[2]) if len(sys.argv) > 2 else 12.0
loads = {}
if os.path.exists(f"{root}/load.jsonl"):
    for line in open(f"{root}/load.jsonl"):
        record = json.loads(line)
        loads[(record["conc"], record["round"])] = record

for conc in (10, 25, 100):
    tti, seg, seg_all, kept, dropped = [], [], [], 0, 0
    for path in sorted(glob.glob(f"{root}/c{conc}-*.jsonl.summary")):
        index = int(os.path.basename(path).split("-")[1].split(".")[0])
        record = loads.get((conc, index))
        if record and max(record["busy_before"], record["busy_after"]) > LIMIT:
            dropped += 1
            continue
        summary = json.loads(open(path).read())
        if summary["succeeded"] != summary["attempted"]:
            dropped += 1
            continue
        kept += 1
        tti.append(summary["tti_p50_ms"])
        seg.append(summary["admitted_to_machine_launched_ms"]["p50"])
        for line in open(path[: -len(".summary")]):
            launch = json.loads(line)
            if launch["ok"]:
                seg_all.append(launch["stages"]["machine_launched"] - launch["stages"]["admitted"])
    if not tti:
        continue
    ordered = sorted(seg_all)
    print(
        json.dumps(
            {
                "concurrency": conc,
                "cohorts": kept,
                "discarded": dropped,
                "tti_p50_median": round(statistics.median(tti), 1),
                "tti_p50_min": min(tti),
                "tti_p50_max": max(tti),
                "tti_spread": round(max(tti) / min(tti), 2),
                "clone_seg_p50_median": round(statistics.median(seg), 2),
                "clone_seg_p50_min": min(seg),
                "clone_seg_p50_max": max(seg),
                "clone_seg_spread": round(max(seg) / max(min(seg), 0.01), 1),
                "clone_seg_per_slot_us": round(1000 * statistics.median(seg) / conc, 1),
                "clone_seg_p99": round(ordered[max(1, -(-99 * len(ordered) // 100)) - 1], 2),
            }
        )
    )
