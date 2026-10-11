#!/usr/bin/env python3
"""Reads the command line's JSON envelopes for the live memory resize check.

The check drives the public command line and needs three facts out of what it prints: the
identity of the machine that was launched, what a command inside the guest wrote, and what a
resize said happened to memory. Each is a subcommand that prints one line, or exits nonzero with
the reason, so a shell stage can record the break instead of swallowing it.

Usage:
  judge.py instance <envelope.json>   the launched machine's 32 character identity
  judge.py stdout   <envelope.json>   the guest command's decoded stdout, when it exited zero
  judge.py memory   <envelope.json>   "<state> <plugged_mb> <total_mb> <booted_mb> <ceiling_mb>"
  judge.py meminfo  <field>           reads /proc/meminfo text on stdin, prints that field in KiB
"""

import base64
import json
import re
import sys


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
        record = json.loads(lines[-1])
    except json.JSONDecodeError as error:
        fail(f"the last line is not an envelope: {error}")
    if record.get("schema") != "soma.cli.v1":
        fail(f"unexpected envelope schema: {record.get('schema')}")
    if record.get("status") != "ok":
        fail(f"the command reported {record.get('status')}: {json.dumps(record.get('error'))[:120]}")
    return record.get("result") or {}


def instance(path: str) -> str:
    identity = envelope(path).get("instance_id")
    if not isinstance(identity, str) or not re.fullmatch(r"[0-9a-f]{32}", identity):
        fail(f"no 32 character instance_id in the launch result: {identity!r}")
    return identity


def stdout(path: str) -> str:
    result = envelope(path)
    code = ((result.get("execution") or {}).get("exited") or {}).get("code")
    if code != 0:
        fail(f"the guest command did not exit zero: {json.dumps(result.get('execution'))[:120]}")
    stream = result.get("stdout") or {}
    if stream.get("encoding") != "base64":
        fail(f"stdout is not base64 encoded: {stream.get('encoding')}")
    return base64.b64decode(stream.get("data", "")).decode("utf-8", "replace")


def memory(path: str) -> str:
    dimension = envelope(path).get("memory")
    if not isinstance(dimension, dict):
        fail("the resize result has no memory dimension")
    needed = ("state", "plugged_mb", "total_mb", "booted_mb", "ceiling_mb")
    missing = [name for name in needed if name not in dimension]
    if missing:
        fail(f"the memory dimension lacks {missing}")
    return " ".join(str(dimension[name]) for name in needed)


def meminfo(field: str) -> str:
    match = re.search(rf"^{re.escape(field)}:\s+(\d+) kB$", sys.stdin.read(), re.MULTILINE)
    if not match:
        fail(f"/proc/meminfo has no {field} line")
    return match.group(1)


def main() -> "None":
    if len(sys.argv) != 3 or sys.argv[1] not in ("instance", "stdout", "memory", "meminfo"):
        fail("usage: judge.py instance|stdout|memory <envelope.json> | meminfo <field>")
    command, argument = sys.argv[1], sys.argv[2]
    handler = {"instance": instance, "stdout": stdout, "memory": memory, "meminfo": meminfo}
    sys.stdout.write(handler[command](argument).rstrip("\n") + "\n")


if __name__ == "__main__":
    main()
