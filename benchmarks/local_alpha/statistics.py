"""Deterministic summary statistics for accepted benchmark timings."""

from __future__ import annotations

from collections.abc import Iterable, Sequence
from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class BenchmarkStatistics:
    accepted_count: int
    failed_count: int
    total_count: int
    success_rate: float
    minimum_ns: int
    maximum_ns: int
    median_ns: int | float
    p95_ns: int
    p99_ns: int

    def as_dict(self) -> dict[str, int | float]:
        return {
            "accepted_count": self.accepted_count,
            "failed_count": self.failed_count,
            "total_count": self.total_count,
            "success_rate": self.success_rate,
            "minimum_ns": self.minimum_ns,
            "maximum_ns": self.maximum_ns,
            "median_ns": self.median_ns,
            "p95_ns": self.p95_ns,
            "p99_ns": self.p99_ns,
        }


def nearest_rank(sorted_samples: Sequence[int], percentile: int) -> int:
    """Return the nearest-rank percentile of an ascending nonempty sample list."""

    if not sorted_samples:
        raise ValueError("at least one accepted sample is required")
    if type(percentile) is not int or not 1 <= percentile <= 100:
        raise ValueError("percentile must be an integer between 1 and 100")
    rank = (percentile * len(sorted_samples) + 99) // 100
    return sorted_samples[rank - 1]


def _median(sorted_samples: list[int]) -> int | float:
    midpoint = len(sorted_samples) // 2
    if len(sorted_samples) % 2 == 1:
        return sorted_samples[midpoint]

    middle_sum = sorted_samples[midpoint - 1] + sorted_samples[midpoint]
    if middle_sum % 2 == 0:
        return middle_sum // 2
    return middle_sum / 2


def summarize(
    samples_ns: Iterable[int],
    *,
    failed_count: int = 0,
) -> BenchmarkStatistics:
    """Summarize accepted nanosecond samples and an explicit failure count."""

    if type(failed_count) is not int or failed_count < 0:
        raise ValueError("failed count must be a nonnegative integer")

    try:
        samples = list(samples_ns)
    except TypeError as error:
        raise ValueError("samples must be an iterable of integers") from error
    if not samples:
        raise ValueError("at least one accepted sample is required")
    if any(type(sample) is not int or sample < 0 for sample in samples):
        raise ValueError("samples must be nonnegative integers")

    samples.sort()
    accepted_count = len(samples)
    total_count = accepted_count + failed_count
    return BenchmarkStatistics(
        accepted_count=accepted_count,
        failed_count=failed_count,
        total_count=total_count,
        success_rate=accepted_count / total_count,
        minimum_ns=samples[0],
        maximum_ns=samples[-1],
        median_ns=_median(samples),
        p95_ns=nearest_rank(samples, 95),
        p99_ns=nearest_rank(samples, 99),
    )
