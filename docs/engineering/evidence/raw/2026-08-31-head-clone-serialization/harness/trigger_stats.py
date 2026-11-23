"""Splits cohorts into fast and slow by cohort wall time and compares what the filesystem did."""

import json
import statistics
import sys

rows = [json.loads(line) for line in open(sys.argv[1])]
rows.sort(key=lambda r: r["cohort_wall_us"])
cut = len(rows) // 4
groups = {"fastest quarter": rows[:cut], "slowest quarter": rows[-cut:], "all": rows}
for name, group in groups.items():
    def med(field):
        return round(statistics.median(r[field] for r in group), 1)

    print(
        json.dumps(
            {
                "group": name,
                "n": len(group),
                "wall_ms": round(med("cohort_wall_us") / 1000, 2),
                "clone_p50_us": med("clone_p50_us"),
                "create_p50_us": med("create_p50_us"),
                "unlink_p50_us": med("unlink_p50_us"),
                "sectors_read": med("sectors_read"),
                "read_ms": med("read_ms"),
                "log_force": med("log_force"),
                "inode_recycle": med("inode_recycle"),
                "load": med("load_before"),
            }
        )
    )
walls = [r["cohort_wall_us"] / 1000 for r in rows]
print(
    json.dumps(
        {
            "cohorts": len(walls),
            "wall_min_ms": round(walls[0], 2),
            "wall_p50_ms": round(statistics.median(walls), 2),
            "wall_p95_ms": round(walls[max(0, -(-95 * len(walls) // 100) - 1)], 2),
            "wall_max_ms": round(walls[-1], 2),
            "spread": round(walls[-1] / walls[0], 2),
        }
    )
)
