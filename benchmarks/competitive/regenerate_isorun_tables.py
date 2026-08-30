#!/usr/bin/env python3
"""Recompute every table cell in the Isorun evidence from the retained records.

A reviewer runs this without an Isorun account. It reads only the redacted JSONL
under docs/evidence/raw/2026-08-30-isorun and prints the cohort table, the
combined distribution, and the cleanup and billing scopes.
"""
import json
import pathlib
import sys

RAW = pathlib.Path("docs/evidence/raw/2026-08-30-isorun")
COHORTS = [
    ("c1.jsonl", "node:22, sequential", 1),
    ("c10.jsonl", "node:22, concurrency 10", 10),
    ("c100.jsonl", "node:22, concurrency 100", 100),
    ("c100b.jsonl", "node:22, concurrency 100, repeat", 100),
    ("alpine-3.20.jsonl", "alpine:3.20, sequential", 1),
    ("busybox-stable-musl.jsonl", "busybox:stable-musl, sequential", 1),
]


def nearest_rank(values, p):
    ordered = sorted(values)
    rank = max(1, -(-p * len(ordered) // 100))
    return ordered[rank - 1]


def succeeded(record):
    """A sample succeeded when its command exited zero and the sandbox was destroyed."""
    return record.get("exit_code") == 0 and (record.get("destroy") or {}).get(
        "status"
    ) == "destroyed"


def main():
    combined, total_cost, attempted, cleaned = [], 0.0, 0, 0
    print(f"| Cohort | concurrency | attempted | succeeded | destroyed | "
          f"min | p50 | p95 | p99 | max |")
    print("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    for name, label, conc in COHORTS:
        path = RAW / name
        if not path.exists():
            print(f"missing cohort file: {path}", file=sys.stderr)
            return 1
        rows = [json.loads(line) for line in path.read_text().splitlines() if line]
        ok = [r for r in rows if succeeded(r)]
        destroyed = [r for r in rows if (r.get("destroy") or {}).get("status") == "destroyed"]
        values = [r["create_ms_server"] for r in ok if r.get("create_ms_server") is not None]
        combined += values
        attempted += len(rows)
        cleaned += len(destroyed)
        total_cost += sum((r.get("destroy") or {}).get("cost_cents", 0.0) for r in rows)
        print(f"| `{label}` | {conc} | {len(rows)} | {len(ok)} | {len(destroyed)} | "
              f"{min(values)} | {nearest_rank(values, 50)} | {nearest_rank(values, 95)} | "
              f"{nearest_rank(values, 99)} | {max(values)} |")
    print(f"| **every cohort** | | {attempted} | {len(combined)} | {cleaned} | "
          f"{min(combined)} | {nearest_rank(combined, 50)} | {nearest_rank(combined, 95)} | "
          f"{nearest_rank(combined, 99)} | {max(combined)} |")
    print()
    print(f"samples at or below 10 ms: {sum(1 for v in combined if v <= 10)}")
    print(f"samples at or below 15 ms: {sum(1 for v in combined if v <= 15)}")
    print(f"samples at or below 22 ms: {sum(1 for v in combined if v <= 22)}")
    print(f"billed cost across these cohorts only: {total_cost:.4f} cents")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
