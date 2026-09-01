"""Combine synchronized host shards into one exact Burst TTI cohort."""

from __future__ import annotations

import argparse
import json
import re
from collections.abc import Mapping, Sequence
from pathlib import Path

from .run import BOUNDARY
from .statistics import computesdk_statistics, qualifies

INSTANCE_ID = re.compile(r"[0-9a-f]{32}")


def combine(documents: Sequence[Mapping[str, object]]) -> dict[str, object]:
    """Merge raw samples without pretending host-local clocks share an origin."""

    if not documents:
        raise ValueError("at least one shard is required")
    samples = []
    boundaries = set()
    release_epochs = set()
    for document in documents:
        if document.get("schema") != "soma.computesdk-burst.v1":
            raise ValueError("shard has an unknown schema")
        shard = document.get("samples")
        if not isinstance(shard, list):
            raise ValueError("shard has no raw sample list")
        samples.extend(shard)
        boundaries.add(document.get("boundary"))
        release_epochs.add(document.get("release_at_epoch_ns"))
    if len(samples) != 100:
        raise ValueError("an exact ComputeSDK burst must contain 100 samples")
    if boundaries != {BOUNDARY}:
        raise ValueError("shards do not share the exact timing boundary")
    if None in release_epochs or len(release_epochs) != 1:
        raise ValueError("shards do not share one release epoch")
    instances = [
        sample.get("instance_id")
        for sample in samples
        if isinstance(sample, Mapping)
    ]
    if not all(isinstance(instance, str) and INSTANCE_ID.fullmatch(instance) for instance in instances):
        raise ValueError("every sample must name one canonical Instance")
    if len(set(instances)) != 100:
        raise ValueError("one Instance appears more than once in the cohort")
    accepted = [
        sample["tti_ns"] / 1_000_000
        for sample in samples
        if isinstance(sample, Mapping)
        and sample.get("command_succeeded") is True
        and isinstance(sample.get("tti_ns"), int)
    ]
    return {
        "schema": "soma.computesdk-burst.combined.v1",
        "boundary": "before create through successful node -v; destroy excluded",
        "attempted": len(samples),
        "succeeded": len(accepted),
        "cleanup_complete": sum(
            isinstance(sample, Mapping) and sample.get("cleanup_complete") is True
            for sample in samples
        ),
        "statistics": computesdk_statistics(accepted),
        "shards": len(documents),
        "release_at_epoch_ns": next(iter(release_epochs)),
        "samples": samples,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("shard", nargs="+", type=Path)
    arguments = parser.parse_args()
    documents = [json.loads(path.read_bytes()) for path in arguments.shard]
    result = combine(documents)
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0 if qualifies(result) else 1


if __name__ == "__main__":
    raise SystemExit(main())
