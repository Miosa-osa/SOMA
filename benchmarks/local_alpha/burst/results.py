"""The burst results file: append-only JSONL and its fail-closed reader."""

from __future__ import annotations

import json
import os
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import IO

from benchmarks.local_alpha.statistics import nearest_rank

from .validation import validate_metadata, validate_samples


RESULTS_SCHEMA = "soma.burst.v2"
LEGACY_RESULTS_SCHEMA = "soma.burst.v1"
READABLE_RESULTS_SCHEMAS = frozenset((LEGACY_RESULTS_SCHEMA, RESULTS_SCHEMA))
MAXIMUM_RECORD_BYTES = 8 * 1024 * 1024
_MERGE_FIELDS = (
    "experiment_class",
    "preparation_class",
    "prepared_before_timer",
    "cache_state",
    "backend",
)


class ResultsWriter:
    """Own one never-overwritten JSONL results file."""

    def __init__(self, destination: Path) -> None:
        descriptor = os.open(
            destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600
        )
        self.destination = destination
        self._stream: IO[bytes] = os.fdopen(descriptor, "wb", buffering=0)

    def __enter__(self) -> "ResultsWriter":
        return self

    def __exit__(self, error_type: object, error: object, traceback: object) -> None:
        if not self._stream.closed:
            self._stream.close()

    def append(self, record: Mapping[str, object]) -> None:
        """Append one durable record carrying the burst results schema."""

        document = {"schema": RESULTS_SCHEMA, **record}
        encoded = (
            json.dumps(document, sort_keys=True, separators=(",", ":")) + "\n"
        ).encode("utf-8")
        if len(encoded) > MAXIMUM_RECORD_BYTES:
            raise ValueError("results record exceeds its capture bound")
        self._stream.write(encoded)
        os.fsync(self._stream.fileno())


@dataclass(frozen=True, slots=True)
class BurstResults:
    """One validated complete burst cohort."""

    path: Path
    metadata: Mapping[str, object]
    samples: tuple[Mapping[str, object], ...]
    completion: Mapping[str, object]

    @property
    def plan(self) -> Mapping[str, object]:
        return self.metadata["plan"]

    @property
    def experiment_class(self) -> str:
        return str(self.plan["experiment_class"])

    @property
    def wall_ns(self) -> int:
        return int(self.completion["wall_ns"])

    @property
    def successful(self) -> tuple[Mapping[str, object], ...]:
        return tuple(sample for sample in self.samples if sample["successful"])

    @property
    def failure_breakdown(self) -> tuple[Mapping[str, object], ...]:
        return tuple(self.completion.get("failure_breakdown") or ())

    @property
    def failed(self) -> tuple[Mapping[str, object], ...]:
        return tuple(sample for sample in self.samples if not sample["successful"])

    def tti_statistics(self) -> dict[str, object]:
        """Summarize accepted time-to-first-command samples by nearest rank."""

        return statistics(
            [int(sample["tti_ns"]) for sample in self.successful],
            failed_count=len(self.failed),
        )


def statistics(values: Sequence[int], *, failed_count: int) -> dict[str, object]:
    """Return nearest-rank statistics over accepted samples and a failure count."""

    if type(failed_count) is not int or failed_count < 0:
        raise ValueError("failed count must be a nonnegative integer")
    total = len(values) + failed_count
    summary: dict[str, object] = {
        "accepted_count": len(values),
        "failed_count": failed_count,
        "total_count": total,
        "success_rate": (len(values) / total) if total else 0.0,
        "percentile_method": "nearest_rank",
    }
    ordered = sorted(values)
    for name, percentile in (("p50_ns", 50), ("p95_ns", 95), ("p99_ns", 99)):
        summary[name] = nearest_rank(ordered, percentile) if ordered else None
    summary["minimum_ns"] = ordered[0] if ordered else None
    summary["maximum_ns"] = ordered[-1] if ordered else None
    return summary


def load_results(path: Path) -> BurstResults:
    """Load one results file, refusing every incomplete or mixed cohort."""

    metadata: Mapping[str, object] | None = None
    completion: Mapping[str, object] | None = None
    samples: list[Mapping[str, object]] = []
    run_ids: set[str] = set()
    schemas: set[str] = set()
    for record in _records(path):
        kind = record.get("record_type")
        run_ids.add(str(record.get("run_id")))
        schemas.add(str(record["schema"]))
        if kind == "run_metadata":
            if metadata is not None:
                raise ValueError(
                    "results file must contain exactly one run metadata record"
                )
            metadata = record
        elif kind == "run_completion":
            if completion is not None:
                raise ValueError(
                    "results file must contain exactly one run completion record"
                )
            completion = record
        elif kind == "sample":
            samples.append(record)
        else:
            raise ValueError("results record type is unknown")
    if metadata is None:
        raise ValueError("results file must contain exactly one run metadata record")
    if completion is None:
        raise ValueError("run is incomplete: no run completion record was retained")
    if len(run_ids) != 1:
        raise ValueError("results records contain multiple run identities")
    if len(schemas) != 1:
        raise ValueError("results records contain multiple schema versions")
    current = next(iter(schemas)) == RESULTS_SCHEMA
    validate_metadata(metadata, require_engine=current)
    validate_samples(metadata, samples, completion, require_attribution=current)
    return BurstResults(
        path=path,
        metadata=metadata,
        samples=tuple(samples),
        completion=completion,
    )


def require_mergeable(cohorts: Sequence[BurstResults]) -> None:
    """Refuse to report cohorts that do not share one experiment declaration."""

    if not cohorts:
        raise ValueError("at least one results file is required")
    first = cohorts[0]
    for cohort in cohorts[1:]:
        for field in _MERGE_FIELDS:
            if cohort.plan[field] != first.plan[field]:
                raise ValueError(f"results merge different {field} values")
        if cohort.metadata["soma"] != first.metadata["soma"]:
            raise ValueError("results merge different SOMA build identities")
        if _host_identity(cohort) != _host_identity(first):
            raise ValueError("results merge different host identities")


def _host_identity(cohort: BurstResults) -> dict[str, object]:
    host = dict(cohort.metadata["host"])
    memory = host.get("memory")
    if isinstance(memory, Mapping):
        host["memory"] = {
            name: value
            for name, value in memory.items()
            if name != "available_at_start"
        }
    return {
        "engine": cohort.metadata.get(
            "engine", {"schema": "soma.engine-settings.unrecorded"}
        ),
        "host": host,
        "backend_probe": cohort.metadata["backend_probe"],
    }


def _records(path: Path):
    with path.open("rb") as stream:
        line_number = 0
        while True:
            encoded = stream.readline(MAXIMUM_RECORD_BYTES + 1)
            if not encoded:
                break
            line_number += 1
            if len(encoded) > MAXIMUM_RECORD_BYTES:
                raise ValueError(f"results line {line_number} exceeds its capture bound")
            try:
                record = json.loads(encoded)
            except (UnicodeDecodeError, json.JSONDecodeError) as error:
                raise ValueError(f"invalid results JSON at line {line_number}") from error
            if (
                not isinstance(record, Mapping)
                or record.get("schema") not in READABLE_RESULTS_SCHEMAS
            ):
                raise ValueError("results record has an unknown schema")
            yield record
        if line_number == 0:
            raise ValueError("results file is empty")
