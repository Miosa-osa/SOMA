"""Per arm phase medians over the quiet cohorts only, with the discarded count shown."""

import json
import statistics
import sys

LIMIT = 10.0
for arm in ("t1-d1", "t4-d1", "t1-d16", "t4-d16"):
    try:
        rows = [json.loads(line) for line in open(f"{sys.argv[1]}/{arm}.cohorts")]
    except OSError:
        continue
    quiet = [r for r in rows if r["load_before"] <= LIMIT and r["load_after"] <= LIMIT]
    if not quiet:
        continue

    def med(field):
        return round(statistics.median(r[field] for r in quiet), 1)

    walls = sorted(r["cohort_wall_us"] for r in quiet)
    print(
        json.dumps(
            {
                "arm": arm,
                "cohorts": len(quiet),
                "discarded": len(rows) - len(quiet),
                "load_max": max(r["load_after"] for r in quiet),
                "create_us": med("create_p50_us"),
                "clone_us": med("clone_p50_us"),
                "verify_us": med("verify_p50_us"),
                "unlink_us": med("unlink_p50_us"),
                "wall_ms": round(statistics.median(walls) / 1000, 2),
                "wall_p95_ms": round(walls[max(0, -(-95 * len(walls) // 100) - 1)] / 1000, 2),
                "wall_max_ms": round(walls[-1] / 1000, 2),
                "per_clone_us": round(statistics.median(walls) / 100, 1),
            }
        )
    )
