#!/usr/bin/env python3
"""Judges one `soma run --format json` envelope and says in one line what it proved.

The end to end check needs more than a zero exit status from the command line. A run counts only
when the envelope reports success, the guest command exited zero, the guest's own output matches
what the caller expected, and the receipt reports every resource the backend owned as released.
Anything short of that is printed as the reason and returned as a nonzero status, so the stage
that called this records the break rather than swallowing it.

Usage: inspect-run.py <envelope.json> <expected-stdout-regex>
"""

import base64
import json
import re
import sys

# Every resource the KVM backend owns for a run. `not_owned` is a truthful disposition for a
# resource this path never created, so it is accepted; `incomplete` never is.
RELEASED = ("complete", "not_owned")
OWNED = ("machine", "memory", "storage", "guest_authority")


def fail(reason: str) -> "None":
    print(reason[:200])
    raise SystemExit(1)


def envelope(path: str) -> "dict":
    try:
        with open(path, "r", encoding="utf-8") as handle:
            lines = [line for line in handle.read().splitlines() if line.strip()]
    except OSError as error:
        fail(f"the envelope could not be read: {error}")
    if not lines:
        fail("the command line wrote no envelope at all")
    try:
        return json.loads(lines[-1])
    except json.JSONDecodeError as error:
        fail(f"the last line is not the machine envelope: {error}")
    return {}


def guest_stdout(result: "dict") -> "str":
    stream = result.get("stdout") or {}
    if stream.get("encoding") != "base64":
        fail(f"stdout is not base64 encoded: {stream.get('encoding')}")
    try:
        return base64.b64decode(stream.get("data", "")).decode("utf-8", "replace").strip()
    except (ValueError, TypeError) as error:
        fail(f"stdout did not decode: {error}")
    return ""


def cleanup_reasons(cleanup: "dict") -> "list":
    reasons = []
    for name in OWNED:
        disposition = cleanup.get(name)
        if disposition not in RELEASED:
            reasons.append(f"{name}={disposition}")
    for name, disposition in (cleanup.get("network") or {}).items():
        if disposition not in RELEASED:
            reasons.append(f"network.{name}={disposition}")
    return reasons


def main() -> "None":
    if len(sys.argv) != 3:
        fail("usage: inspect-run.py <envelope.json> <expected-stdout-regex>")
    record = envelope(sys.argv[1])
    if record.get("schema") != "soma.cli.v1":
        fail(f"unexpected envelope schema: {record.get('schema')}")
    if record.get("status") != "ok":
        fail(f"the run reported {record.get('status')}: {json.dumps(record.get('error'))[:120]}")
    result = record.get("result") or {}
    code = ((result.get("execution") or {}).get("exited") or {}).get("code")
    if code != 0:
        fail(f"the guest command did not exit zero: {json.dumps(result.get('execution'))[:120]}")
    output = guest_stdout(result)
    if not re.search(sys.argv[2], output):
        fail(f"guest stdout {output[:80]!r} does not match {sys.argv[2]!r}")
    receipt = record.get("receipt") or {}
    reasons = cleanup_reasons(receipt.get("cleanup") or {})
    if reasons:
        fail("the receipt reports unreleased resources: " + ", ".join(reasons))
    milestones = {m["kind"]: m["elapsed_ns"] / 1e6 for m in receipt.get("milestones") or []}
    isolation = (receipt.get("isolation") or {}).get("value")
    print(
        f"stdout={output[:40]!r} exit=0 isolation={isolation} "
        f"ready={milestones.get('ready', float('nan')):.1f}ms "
        f"finished={milestones.get('cleanup_finished', float('nan')):.1f}ms "
        f"instance={result.get('instance_id')}"
    )


if __name__ == "__main__":
    main()
