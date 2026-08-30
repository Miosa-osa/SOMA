"""The burst results file: append-only JSONL and its fail-closed reader."""

from __future__ import annotations

import json
import os
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import IO

from benchmarks.local_alpha.statistics import nearest_rank

from .plan import EXPERIMENT_CLASSES


RESULTS_SCHEMA = "soma.burst.v1"
MAXIMUM_RECORD_BYTES = 8 * 1024 * 1024
_METADATA_FIELDS = ("run_id", "started_at_utc", "plan", "soma", "host", "backend_probe")
_PLAN_FIELDS = (
    "experiment_class",
    "preparation_class",
    "prepared_before_timer",
    "cache_state",
    "backend",
    "image",
    "command",
    "network_policy",
    "shape",
    "iterations",
    "concurrency",
    "bursts",
    "timeout_ms",
    "max_output_bytes",
    "excluded_work",
)
_SOMA_FIELDS = ("git_revision", "worktree_clean", "build_manifest")
_HOST_FIELDS = ("kernel", "cpu", "memory", "storage", "kvm")
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
    for record in _records(path):
        kind = record.get("record_type")
        run_ids.add(str(record.get("run_id")))
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
    _validate_metadata(metadata)
    _validate_samples(metadata, samples, completion)
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
    return {"host": host, "backend_probe": cohort.metadata["backend_probe"]}


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
            if not isinstance(record, Mapping) or record.get("schema") != RESULTS_SCHEMA:
                raise ValueError("results record has an unknown schema")
            yield record
        if line_number == 0:
            raise ValueError("results file is empty")


def _validate_metadata(metadata: Mapping[str, object]) -> None:
    _require(metadata, _METADATA_FIELDS, "run metadata")
    plan = metadata["plan"]
    soma = metadata["soma"]
    host = metadata["host"]
    _require(plan, _PLAN_FIELDS, "run metadata plan")
    _require(soma, _SOMA_FIELDS, "run metadata soma identity")
    _require(host, _HOST_FIELDS, "run metadata host identity")
    if plan["experiment_class"] not in EXPERIMENT_CLASSES:
        raise ValueError("run metadata declares an unknown experiment class")
    if plan["preparation_class"] != plan["experiment_class"]:
        raise ValueError("run metadata preparation class contradicts its experiment class")
    prepared = plan["prepared_before_timer"]
    if not isinstance(prepared, list):
        raise ValueError("run metadata preparation must be a list")
    if plan["experiment_class"] != "cold-generation-build" and not prepared:
        raise ValueError(
            f"class {plan['experiment_class']} must record what was prepared "
            "before the timer"
        )
    if not metadata["plan"]["excluded_work"]:
        raise ValueError("run metadata must name the work excluded from the timer")


def _validate_samples(
    metadata: Mapping[str, object],
    samples: Sequence[Mapping[str, object]],
    completion: Mapping[str, object],
) -> None:
    plan = metadata["plan"]
    iterations = plan["iterations"]
    if completion.get("attempted") != len(samples) or len(samples) != iterations:
        raise ValueError(
            f"run is incomplete: {len(samples)} of {iterations} samples were retained"
        )
    repetitions = sorted(int(sample["repetition"]) for sample in samples)
    if repetitions != list(range(1, iterations + 1)):
        raise ValueError("sample repetitions must cover the cohort exactly once")
    for sample in samples:
        if sample.get("experiment_class") != plan["experiment_class"]:
            raise ValueError("results merge different experiment classes")
        if sample["successful"]:
            _require_command(sample)
        elif not sample.get("failures"):
            raise ValueError("an unsuccessful sample lacks a typed failure reason")


def _require_command(sample: Mapping[str, object]) -> None:
    command = sample.get("command")
    if (
        not isinstance(command, Mapping)
        or command.get("status") != "exited"
        or command.get("exit_code") != 0
        or not isinstance(command.get("stdout"), Mapping)
        or type(sample.get("tti_ns")) is not int
        or sample["tti_ns"] < 0
        or not sample.get("cleanup_complete")
    ):
        raise ValueError("a successful sample lacks workload command evidence")


def _require(value: object, fields: Sequence[str], label: str) -> None:
    if not isinstance(value, Mapping):
        raise ValueError(f"{label} must be an object")
    for field in fields:
        if value.get(field) in (None, "", {}):
            raise ValueError(f"{label} is missing required field: {field}")
