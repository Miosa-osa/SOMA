"""Fail-closed extraction of monotonic durations from SOMA receipts."""

from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class Milestone:
    kind: str
    elapsed_ns: int


_KIND_RANK = {
    "accepted": 0,
    "workload_resolved": 1,
    "inspected": 1,
    "admitted": 2,
    "machine_launched": 3,
    "ready": 4,
    "command_started": 5,
    "command_finished": 6,
    "failure_observed": 7,
    "cleanup_started": 8,
    "cleanup_finished": 9,
}
_COMPLETE_RUN_KINDS = (
    "accepted",
    "workload_resolved",
    "admitted",
    "machine_launched",
    "ready",
    "command_started",
    "command_finished",
    "cleanup_started",
    "cleanup_finished",
)


def parse_milestones(receipt: Mapping[str, object]) -> tuple[Milestone, ...]:
    """Validate and return a receipt's unique, ordered milestones."""

    if not isinstance(receipt, Mapping) or "milestones" not in receipt:
        raise ValueError("receipt must contain milestones")
    raw_milestones = receipt["milestones"]
    if type(raw_milestones) is not list or not raw_milestones:
        raise ValueError("receipt milestones must be a nonempty list")

    parsed: list[Milestone] = []
    seen: set[str] = set()
    previous_rank = -1
    previous_elapsed_ns = -1
    for raw in raw_milestones:
        if not isinstance(raw, Mapping) or set(raw) != {"kind", "elapsed_ns"}:
            raise ValueError("each milestone must contain only kind and elapsed_ns")
        kind = raw["kind"]
        elapsed_ns = raw["elapsed_ns"]
        if type(kind) is not str or kind not in _KIND_RANK:
            raise ValueError("milestone kind is unknown")
        if type(elapsed_ns) is not int or elapsed_ns < 0:
            raise ValueError("milestone elapsed_ns must be a nonnegative integer")
        if kind in seen:
            raise ValueError("milestone kinds must be unique")

        rank = _KIND_RANK[kind]
        if rank <= previous_rank:
            raise ValueError("milestone kinds are out of order")
        if elapsed_ns < previous_elapsed_ns:
            raise ValueError("milestone elapsed_ns values regress")

        parsed.append(Milestone(kind=kind, elapsed_ns=elapsed_ns))
        seen.add(kind)
        previous_rank = rank
        previous_elapsed_ns = elapsed_ns
    return tuple(parsed)


def _index(milestones: tuple[Milestone, ...]) -> dict[str, tuple[int, int]]:
    return {
        milestone.kind: (position, milestone.elapsed_ns)
        for position, milestone in enumerate(milestones)
    }


def _duration(index: Mapping[str, tuple[int, int]], start: str, end: str) -> int:
    if type(start) is not str or type(end) is not str:
        raise ValueError("duration boundaries must be milestone names")
    if start not in index or end not in index:
        raise ValueError("duration boundary milestone is missing")
    start_position, start_ns = index[start]
    end_position, end_ns = index[end]
    if start_position > end_position:
        raise ValueError("duration boundaries are reversed")
    return end_ns - start_ns


def duration_ns(receipt: Mapping[str, object], start: str, end: str) -> int:
    """Return elapsed nanoseconds between two receipt milestones."""

    return _duration(_index(parse_milestones(receipt)), start, end)


def run_metrics(receipt: Mapping[str, object]) -> dict[str, int]:
    """Return phase durations for one complete successful one-shot run."""

    milestones = parse_milestones(receipt)
    if tuple(milestone.kind for milestone in milestones) != _COMPLETE_RUN_KINDS:
        raise ValueError("complete one-shot run milestones are required")
    index = _index(milestones)
    return {
        "image_resolve": _duration(index, "accepted", "workload_resolved"),
        "launch_ready": _duration(index, "admitted", "ready"),
        "admitted_to_command_finished": _duration(
            index, "admitted", "command_finished"
        ),
        "ready_to_command_finished": _duration(index, "ready", "command_finished"),
        "command": _duration(index, "command_started", "command_finished"),
        "cleanup": _duration(index, "cleanup_started", "cleanup_finished"),
        "request_total": _duration(index, "accepted", "cleanup_finished"),
    }
