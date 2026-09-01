"""Barrier release and raw result assembly for exact public-API Burst TTI."""

from __future__ import annotations

import time
from collections.abc import Callable, Sequence
from concurrent.futures import ThreadPoolExecutor
from threading import Barrier

from .client import ApiClient
from .slot import execute_slot
from .statistics import computesdk_statistics

BOUNDARY = "before create through successful node -v; destroy excluded"


def run(
    endpoints: Sequence[str],
    *,
    tenant: str,
    release_at_epoch_ns: int | None = None,
    client_factory: Callable[[str, str], object] = ApiClient,
) -> dict[str, object]:
    """Open every assigned slot together and report raw and ComputeSDK statistics."""

    if not endpoints:
        raise ValueError("at least one endpoint slot is required")
    barrier = Barrier(len(endpoints))

    def worker(endpoint: str) -> dict[str, object]:
        sample = execute_slot(
            client_factory(endpoint, tenant),
            barrier,
            release_at_epoch_ns=release_at_epoch_ns,
        )
        sample["endpoint"] = endpoint
        return sample

    with ThreadPoolExecutor(max_workers=len(endpoints)) as pool:
        samples = list(pool.map(worker, endpoints))
    accepted = [
        sample["tti_ns"] / 1_000_000
        for sample in samples
        if sample["command_succeeded"] and isinstance(sample["tti_ns"], int)
    ]
    starts = [sample["started_ns"] for sample in samples if sample["started_ns"]]
    finishes = [sample["tti_finished_ns"] for sample in samples if sample["tti_finished_ns"]]
    origin = min(starts) if starts else None
    return {
        "schema": "soma.computesdk-burst.v1",
        "measured_at_unix_ns": time.time_ns(),
        "boundary": BOUNDARY,
        "release_at_epoch_ns": release_at_epoch_ns,
        "attempted": len(samples),
        "succeeded": len(accepted),
        "cleanup_complete": sum(bool(sample["cleanup_complete"]) for sample in samples),
        "wall_clock_ms": _since(origin, max(finishes) if finishes else None),
        "time_to_first_ready_ms": _since(origin, min(finishes) if finishes else None),
        "statistics": computesdk_statistics(accepted),
        "samples": samples,
    }


def _since(origin: int | None, finish: int | None) -> float | None:
    return None if origin is None or finish is None else (finish - origin) / 1_000_000
