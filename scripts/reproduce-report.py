#!/usr/bin/env python3
"""Turns one directory of `soma --format json run` receipts into a measured result.

Every launch either printed the text the caller said it must print, or it did not. A run where
some launches failed is not a slower run, it is a different run, so this exits nonzero and names
the first cause rather than quoting a percentile over whatever survived.

Usage: reproduce-report.py <raw-dir> <expected-text> <image> <vcpus> <memory-mib> <concurrency>
"""

import base64
import json
import pathlib
import statistics
import sys

# A restored machine reaches ready in tens of milliseconds. A prepared entry with no snapshot
# cold boots instead, which is not an error anywhere, so the segment length is the only signal
# in the receipt itself that the number below measures something other than a restore.
COLD_BOOT_READY_MS = 200.0
STAGES = ("admitted", "machine_launched", "ready", "command_finished")


def percentile(values, share):
    if not values:
        return None
    rank = max(1, -(-share * len(values) // 100))
    return round(values[rank - 1], 2)


def read_receipt(path, expected):
    """Returns (milestones, failure). Exactly one of the two is None."""
    try:
        text = path.read_text(encoding="utf-8", errors="replace").strip()
    except OSError as error:
        return None, f"{path.name}: unreadable ({error})"
    if not text:
        return None, f"{path.name}: the run wrote nothing, so it never produced a receipt"
    try:
        record = json.loads(text.splitlines()[-1])
    except json.JSONDecodeError as error:
        return None, f"{path.name}: the last output line is not JSON ({error})"
    if "receipt" not in record:
        return None, f"{path.name}: the record carries no receipt; the launch was refused"
    result = record.get("result")
    if not result:
        return None, f"{path.name}: the receipt carries no command result"
    raw = result.get("stdout", {}).get("data", "")
    out = base64.b64decode(raw).decode("utf-8", errors="replace")
    if expected not in out:
        head = " ".join(out.split())[:120]
        return None, f"{path.name}: the guest never printed {expected!r}; it printed {head!r}"
    milestones = {
        m["kind"]: m["elapsed_ns"] / 1e6 for m in record["receipt"].get("milestones", [])
    }
    if "command_finished" not in milestones:
        return None, f"{path.name}: the receipt has no command_finished milestone"
    return milestones, None


def stage_deltas(samples):
    deltas = {}
    previous = 0.0
    for stage in STAGES:
        values = [s[stage] for s in samples if stage in s]
        if not values:
            continue
        median = statistics.median(values)
        deltas[stage] = round(median - previous, 2)
        previous = median
    return deltas


def main(argv):
    if len(argv) != 7:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    raw, expected, image, vcpus, memory_mib, concurrency = argv[1:7]
    paths = sorted(pathlib.Path(raw).glob("*.json"))
    if not paths:
        print("reproduce: no launch wrote a receipt at all", file=sys.stderr)
        return 1

    samples, failures = [], []
    for path in paths:
        milestones, failure = read_receipt(path, expected)
        if failure is None:
            samples.append(milestones)
        else:
            failures.append(failure)

    times = sorted(s["command_finished"] for s in samples)
    deltas = stage_deltas(samples)
    print("")
    print(f"  image        {image}")
    print(f"  shape        {vcpus} vCPU, {memory_mib} MiB, concurrency {concurrency}")
    print(f"  launches     {len(samples)} of {len(paths)} succeeded")
    if times:
        print(f"  time to first command  p50 {percentile(times, 50)} ms, "
              f"min {round(times[0], 2)} ms, max {round(times[-1], 2)} ms")
        print("  segment medians        " + ", ".join(
            f"{stage} {value} ms" for stage, value in deltas.items()))

    refused = 0
    if failures:
        refused = 1
        print("")
        print(f"reproduce: {len(failures)} of {len(paths)} launches did not produce a result:",
              file=sys.stderr)
        for failure in failures[:5]:
            print(f"  {failure}", file=sys.stderr)
        if len(failures) > 5:
            print(f"  and {len(failures) - 5} more", file=sys.stderr)
        print("  A partial run is not a slower run. Fix the cause before quoting a figure.",
              file=sys.stderr)

    ready = deltas.get("ready")
    if ready is not None and ready > COLD_BOOT_READY_MS:
        refused = 1
        print("")
        print(f"reproduce: the ready segment is {ready} ms, which is a cold boot rather than a",
              file=sys.stderr)
        print("  restore. The prepared entry has no usable snapshot beside it, so this number",
              file=sys.stderr)
        print("  measures booting a kernel and is not a restore result.", file=sys.stderr)
    return refused


if __name__ == "__main__":
    sys.exit(main(sys.argv))
