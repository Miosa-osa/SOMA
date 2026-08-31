"""Barrier-released burst execution over one validated plan."""

from __future__ import annotations

import threading
import time
from collections.abc import Callable, Mapping
from concurrent.futures import ThreadPoolExecutor
from datetime import UTC, datetime
from pathlib import Path

from benchmarks.local_alpha.runner.identities import IdentityGenerator

from .attribution import failure_breakdown, shape_disagreement
from .invocation import OPERATIONS
from .plan import BurstPlan
from .results import ResultsWriter, statistics
from .slot import BOUNDARY, BurstSample, execute_slot


BARRIER_TIMEOUT_SECONDS = 300.0


def run_burst(
    plan: BurstPlan,
    *,
    soma_binary: Path,
    state_root: Path,
    environment: Mapping[str, str],
    metadata: Mapping[str, object],
    results_path: Path,
    identities: IdentityGenerator | None = None,
    slot: Callable[..., BurstSample] = execute_slot,
    clock: Callable[[], int] = time.perf_counter_ns,
) -> dict[str, object]:
    """Open every slot of each burst at once and retain every attempted sample."""

    identities = identities or IdentityGenerator()
    run_id = str(metadata["run_id"])
    samples: list[BurstSample] = []
    with ResultsWriter(results_path) as writer:
        writer.append({"record_type": "run_metadata", **metadata})
        wall_started_ns = clock()
        for burst_index in range(plan.bursts):
            for slot_index, sample in enumerate(
                _one_burst(
                    plan,
                    identities=identities,
                    slot=slot,
                    soma_binary=soma_binary,
                    state_root=state_root,
                    environment=environment,
                )
            ):
                samples.append(sample)
                writer.append(
                    sample.as_record(
                        plan=plan,
                        run_id=run_id,
                        repetition=len(samples),
                        burst_index=burst_index,
                        slot_index=slot_index,
                    )
                )
        wall_ns = clock() - wall_started_ns
        summary = _summary(plan, run_id, samples, wall_ns)
        writer.append(summary)
    return summary


def _one_burst(
    plan: BurstPlan,
    *,
    identities: IdentityGenerator,
    slot: Callable[..., BurstSample],
    soma_binary: Path,
    state_root: Path,
    environment: Mapping[str, str],
) -> list[BurstSample]:
    assignments = [
        (
            identities.new(),
            {operation: identities.new() for operation in OPERATIONS},
        )
        for _ in range(plan.concurrency)
    ]
    barrier = threading.Barrier(plan.concurrency)

    def worker(instance_id: str, operation_ids: dict[str, str]) -> BurstSample:
        barrier.wait(timeout=BARRIER_TIMEOUT_SECONDS)
        return slot(
            plan,
            soma_binary=soma_binary,
            state_root=state_root,
            environment=environment,
            instance_id=instance_id,
            operation_ids=operation_ids,
        )

    with ThreadPoolExecutor(max_workers=plan.concurrency) as pool:
        futures = [
            pool.submit(worker, instance_id, operation_ids)
            for instance_id, operation_ids in assignments
        ]
        return [future.result() for future in futures]


def _summary(
    plan: BurstPlan,
    run_id: str,
    samples: list[BurstSample],
    wall_ns: int,
) -> dict[str, object]:
    successful = [sample for sample in samples if sample.successful]
    breakdown = failure_breakdown([sample.failures for sample in samples])
    shapes = sorted(
        {
            note
            for note in (shape_disagreement(sample.observed) for sample in samples)
            if note
        }
    )
    return {
        "record_type": "run_completion",
        "run_id": run_id,
        "finished_at_utc": datetime.now(UTC).isoformat(),
        "experiment_class": plan.experiment_class,
        "attempted": len(samples),
        "wall_ns": wall_ns,
        "wall_includes": "every launch, command, and destruction of the whole run",
        "boundary": BOUNDARY,
        "command_succeeded_count": sum(
            sample.command_succeeded for sample in samples
        ),
        "cleanup_complete_count": sum(sample.cleanup_complete for sample in samples),
        # Why the run scored what it scored, in the run's own completion record. A count with
        # no attributable reason is what made a zero unreadable without opening every slot.
        "failure_breakdown": breakdown,
        # A launch answers ok for a shape it was never shown to have delivered. Saying so here
        # is the difference between a measurement and a measurement of something else.
        "shape_disagreements": shapes,
        "tti": statistics(
            [int(sample.tti_ns) for sample in successful if sample.tti_ns is not None],
            failed_count=len(samples) - len(successful),
        ),
    }
