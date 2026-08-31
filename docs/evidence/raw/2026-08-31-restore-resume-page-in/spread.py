"""Where in the captured image the pages a restored guest touches actually live.

Huge pages only help if the touched pages cluster: a 2 MiB backing page is a win when a
guest touches many of its 512 small pages and a loss when it touches one. This reads
`/proc/<pid>/pagemap` over the memory mapping and reports, per 2 MiB region, how many of its
512 pages are present at all and how many are private copies.
"""

import json
import os
import struct
import sys

pid, want_kb = sys.argv[1], int(sys.argv[2])
path = None
start = end = 0
for line in open(f"/proc/{pid}/maps"):
    fields = line.split()
    lo, _, hi = fields[0].partition("-")
    lo, hi = int(lo, 16), int(hi, 16)
    if (hi - lo) // 1024 == want_kb and fields[-1].endswith("memory.raw"):
        if hi - lo > end - start:
            start, end, path = lo, hi, fields[-1]

if path is None:
    print(json.dumps({"error": "no memory mapping found"}))
    raise SystemExit(1)

PAGE = 4096
HUGE = 2 * 1024 * 1024
pages = (end - start) // PAGE
with open(f"/proc/{pid}/pagemap", "rb", buffering=0) as pagemap:
    pagemap.seek(start // PAGE * 8)
    data = pagemap.read(pages * 8)

present_per_region = {}
present = 0
for index in range(len(data) // 8):
    entry = struct.unpack_from("<Q", data, index * 8)[0]
    if entry >> 63 & 1:
        present += 1
        present_per_region[index * PAGE // HUGE] = present_per_region.get(
            index * PAGE // HUGE, 0) + 1

occupied = sorted(present_per_region.values(), reverse=True)
print(json.dumps({
    "mapping_mib": (end - start) >> 20,
    "present_pages": present,
    "present_mib": round(present * PAGE / (1 << 20), 2),
    "regions_2mib_touched": len(occupied),
    "regions_2mib_total": (end - start) // HUGE,
    "pages_per_touched_region_mean": round(present / len(occupied), 1) if occupied else 0,
    "occupancy_histogram": {
        "1_to_8": sum(1 for v in occupied if v <= 8),
        "9_to_64": sum(1 for v in occupied if 8 < v <= 64),
        "65_to_256": sum(1 for v in occupied if 64 < v <= 256),
        "257_to_512": sum(1 for v in occupied if v > 256),
    },
    "huge_pages_if_promoted_mib": round(len(occupied) * 2, 1),
}))
