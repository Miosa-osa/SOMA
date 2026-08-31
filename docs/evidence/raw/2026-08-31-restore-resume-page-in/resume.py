"""Medians of the resume window's parts across one set of timeline files."""

import glob
import json
import os
import statistics
import sys

directory, label = sys.argv[1], sys.argv[2]
STEPS = [
    ("map memory to prefault done", "MapMemory", "PrefaultMemory"),
    ("armed to first entry", "RunStart", "FirstRunEntered"),
    ("first KVM_RUN call", "FirstRunEntered", "FirstRunReturned"),
    ("first return to launch page erased", "FirstRunReturned", "LaunchPageConsumed"),
    ("armed to launch page erased", "RunStart", "LaunchPageConsumed"),
    ("launch page erased to vsock", "LaunchPageConsumed", "VsockConnected"),
    ("vsock to handshake", "VsockConnected", "Handshake"),
    ("handshake to ready", "Handshake", "Ready"),
    ("armed to ready", "RunStart", "Ready"),
    ("accepted to ready", None, "Ready"),
]

samples = {name: [] for name, _, _ in STEPS}
for path in sorted(glob.glob(os.path.join(directory, "*.json"))):
    marks = json.loads(open(path).read())["milestones_ns"]
    for name, start, end in STEPS:
        if end not in marks or (start is not None and start not in marks):
            continue
        samples[name].append((marks[end] - (marks[start] if start else 0)) / 1e6)

print(f"== {label}: {len(glob.glob(os.path.join(directory, '*.json')))} samples")
for name, _, _ in STEPS:
    values = sorted(samples[name])
    if not values:
        print(f"  {name:<38} absent")
        continue
    print(f"  {name:<38} n={len(values):<3} p50={statistics.median(values):7.3f} "
          f"min={values[0]:7.3f} max={values[-1]:7.3f}")
