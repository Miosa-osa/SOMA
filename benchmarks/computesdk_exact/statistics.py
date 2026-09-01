"""ComputeSDK-compatible trimming and percentile statistics."""

from __future__ import annotations

import math
import statistics
from collections.abc import Mapping, Sequence


def qualifies(result: Mapping[str, object]) -> bool:
    """Require the exact cohort, every command, and every excluded cleanup."""

    return (
        result.get("attempted") == 100
        and result.get("succeeded") == 100
        and result.get("cleanup_complete") == 100
    )


def computesdk_statistics(values: Sequence[float]) -> dict[str, float | int | None]:
    """Apply ComputeSDK's five-percent two-sided trim and nearest-rank tails."""

    ordered = sorted(values)
    if not ordered:
        return {
            "raw_count": 0, "trimmed_count": 0, "trim_each_side": 0,
            "median_ms": None, "p95_ms": None, "p99_ms": None,
            "raw_minimum_ms": None, "raw_maximum_ms": None,
        }
    trim = math.floor(len(ordered) * 0.05)
    trimmed = ordered[trim : len(ordered) - trim] if trim else ordered
    return {
        "raw_count": len(ordered),
        "trimmed_count": len(trimmed),
        "trim_each_side": trim,
        "median_ms": statistics.median(trimmed),
        "p95_ms": _nearest_rank(trimmed, 0.95),
        "p99_ms": _nearest_rank(trimmed, 0.99),
        "raw_minimum_ms": ordered[0],
        "raw_maximum_ms": ordered[-1],
    }


def _nearest_rank(ordered: Sequence[float], percentile: float) -> float:
    index = max(0, math.ceil(percentile * len(ordered)) - 1)
    return ordered[min(index, len(ordered) - 1)]
