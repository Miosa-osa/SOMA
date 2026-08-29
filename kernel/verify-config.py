#!/usr/bin/env python3
"""Fail closed if a Linux .config drifted from the pinned SOMA configuration.

Usage:
    verify-config.py --pinned PINNED --final FINAL --required REQUIRED

Two independent checks run:

1. Every symbol in PINNED must have the same value in FINAL.  `make olddefconfig`
   is allowed to append symbols that are absent from PINNED, but it must not flip
   or drop any pinned value.
2. Every symbol in REQUIRED (the machine-contract list) must hold in FINAL.

Only the Python standard library is used.
"""

import argparse
import re
import sys

SET_RE = re.compile(r"^(CONFIG_[A-Za-z0-9_]+)=(.*)$")
UNSET_RE = re.compile(r"^# (CONFIG_[A-Za-z0-9_]+) is not set$")


def parse(path):
    values = {}
    with open(path, encoding="utf-8") as handle:
        for raw in handle:
            line = raw.rstrip("\n")
            match = SET_RE.match(line)
            if match:
                values[match.group(1)] = match.group(2)
                continue
            match = UNSET_RE.match(line)
            if match:
                values[match.group(1)] = "n"
    return values


def compare(label, expected, final, treat_absent_as_n):
    problems = []
    for symbol, want in sorted(expected.items()):
        got = final.get(symbol)
        if got is None and want == "n" and treat_absent_as_n:
            continue
        if got != want:
            shown = "<absent>" if got is None else got
            problems.append(f"{label}: {symbol} expected {want} but final has {shown}")
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pinned", required=True)
    parser.add_argument("--final", required=True)
    parser.add_argument("--required", required=True)
    args = parser.parse_args()

    pinned = parse(args.pinned)
    final = parse(args.final)
    required = parse(args.required)

    problems = compare("pinned", pinned, final, treat_absent_as_n=False)
    problems += compare("required", required, final, treat_absent_as_n=True)

    if problems:
        for problem in problems:
            print(problem, file=sys.stderr)
        print(f"verify-config: FAIL ({len(problems)} mismatches)", file=sys.stderr)
        return 1
    print(
        f"verify-config: OK ({len(pinned)} pinned symbols unchanged, "
        f"{len(required)} required symbols hold, {len(final)} symbols in final config)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
