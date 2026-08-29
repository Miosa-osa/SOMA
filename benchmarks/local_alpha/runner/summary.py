"""Aggregate accepted sample timings with explicit failure counts."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import dataclass

from benchmarks.local_alpha.statistics import summarize

from .model import SampleOutcome


_PREPARATION_CLASSES = {
    "on_demand",
    "prepared_worker",
    "paused_lease",
    "ready_lease",
}


@dataclass(frozen=True, slots=True)
class CompactSample:
    duration_ns: int
    boundary: str
    accepted: bool
    cleanup_validated: bool
    receipt_metrics_ns: dict[str, int]
    preparation_classes: tuple[str, ...]

    @classmethod
    def from_outcome(cls, outcome: SampleOutcome) -> "CompactSample":
        classes = sorted(
            {
                value
                for operation in outcome.operations
                if isinstance((value := operation.get("preparation_class")), str)
            }
        )
        if any(value not in _PREPARATION_CLASSES for value in classes):
            raise ValueError("receipt preparation class is unknown")
        return cls(
            duration_ns=outcome.duration_ns,
            boundary=outcome.boundary,
            accepted=outcome.accepted,
            cleanup_validated=outcome.cleanup_validated,
            receipt_metrics_ns=dict(outcome.receipt_metrics_ns),
            preparation_classes=tuple(classes),
        )


def observed_preparation_class(receipt: Mapping[str, object]) -> str | None:
    preparation = receipt.get("preparation")
    if preparation is None:
        return None
    if not isinstance(preparation, Mapping):
        raise ValueError("receipt preparation observation is malformed")
    state = preparation.get("state")
    if state == "unavailable":
        return None
    value = preparation.get("value")
    if (
        state != "observed"
        or not isinstance(value, str)
        or value not in _PREPARATION_CLASSES
    ):
        raise ValueError("receipt preparation observation is malformed")
    return value


def summary_preparation_class(samples: Sequence[CompactSample]) -> str | None:
    observed = {
        preparation
        for sample in samples
        for preparation in sample.preparation_classes
    }
    if len(observed) > 1:
        raise ValueError("observed receipt preparation classes differ")
    return next(iter(observed), None)


def metric_summaries(samples: Sequence[CompactSample]) -> dict[str, object]:
    summary_preparation_class(samples)
    total = len(samples)
    accepted = tuple(sample for sample in samples if sample.accepted)
    external = _statistics(
        [sample.duration_ns for sample in accepted],
        total,
    )
    names = sorted(
        {
            name
            for sample in accepted
            for name in sample.receipt_metrics_ns
        }
    )
    receipt = {
        name: _statistics(
            [
                sample.receipt_metrics_ns[name]
                for sample in accepted
                if name in sample.receipt_metrics_ns
            ],
            total,
        )
        for name in names
    }
    return {"external_tti": external, "receipt": receipt}


def _statistics(values: list[int], total: int) -> dict[str, object]:
    if values:
        return summarize(values, failed_count=total - len(values)).as_dict()
    return {
        "accepted_count": 0,
        "failed_count": total,
        "total_count": total,
        "success_rate": 0.0,
        "minimum_ns": None,
        "maximum_ns": None,
        "median_ns": None,
        "p95_ns": None,
        "p99_ns": None,
    }
