"""Command line for exact SOMA public-API Burst TTI qualification."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

from .run import run
from .statistics import qualifies


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--endpoint", action="append", required=True, metavar="URL=COUNT")
    parser.add_argument("--tenant", default="qualification")
    parser.add_argument("--release-at-epoch-ns", type=int)
    parser.add_argument("--output", type=Path)
    arguments = parser.parse_args()
    result = run(
        _expand(arguments.endpoint),
        tenant=arguments.tenant,
        release_at_epoch_ns=arguments.release_at_epoch_ns,
    )
    encoded = json.dumps(result, sort_keys=True, separators=(",", ":")) + "\n"
    if arguments.output is not None:
        descriptor = os.open(arguments.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
            stream.write(encoded)
    print(encoded, end="")
    return 0 if qualifies(result) else 1


def _expand(specifications: list[str]) -> list[str]:
    endpoints = []
    for specification in specifications:
        endpoint, separator, raw_count = specification.rpartition("=")
        if not separator:
            raise ValueError("endpoint assignment must be URL=COUNT")
        count = int(raw_count)
        if count <= 0:
            raise ValueError("endpoint slot count must be positive")
        endpoints.extend([endpoint] * count)
    return endpoints


if __name__ == "__main__":
    raise SystemExit(main())
