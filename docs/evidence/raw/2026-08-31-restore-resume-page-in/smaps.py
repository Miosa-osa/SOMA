"""Report the page-in state of one restored sandbox's captured memory mapping.

`Rss` on the VMA whose size is exactly the guest's memory counts the pages the guest has
faulted in since the restore mapped the image; `Private_Dirty` counts the copy-on-write
copies among them, which are the expensive half.
"""

import json
import sys

pid, want_kb, label = sys.argv[1], int(sys.argv[2]), sys.argv[3]
blocks, current = [], None
for line in open(f"/proc/{pid}/smaps"):
    head = line.split(maxsplit=1)[0]
    if head.endswith(":"):
        key = head[:-1]
        rest = line.split()
        if len(rest) >= 3 and rest[2] == "kB":
            current[key] = int(rest[1])
    else:
        current = {"header": line.strip()}
        blocks.append(current)

match = [b for b in blocks if b.get("Size") == want_kb]
match.sort(key=lambda b: -b.get("Rss", 0))
best = match[0] if match else {}
stat = open(f"/proc/{pid}/stat").read().split()
print(json.dumps({
    "label": label,
    "vma_kb": best.get("Size"),
    "rss_kb": best.get("Rss"),
    "private_dirty_kb": best.get("Private_Dirty"),
    "shared_clean_kb": best.get("Shared_Clean"),
    "faulted_pages": (best.get("Rss") or 0) // 4,
    "cow_pages": (best.get("Private_Dirty") or 0) // 4,
    "candidate_vmas": len(match),
    "process_minor_faults": int(stat[9]),
    "process_major_faults": int(stat[11]),
}))
