"""Stage durations derived from the CLI receipt milestones of one cohort."""

from __future__ import annotations

from collections.abc import Mapping, Sequence


STAGE_DEFINITIONS = (
    ("launch: workload resolution", "launch", "accepted", "workload_resolved"),
    ("launch: admission", "launch", "workload_resolved", "admitted"),
    ("launch: machine creation", "launch", "admitted", "machine_launched"),
    ("launch: readiness", "launch", "machine_launched", "ready"),
    ("launch: facade total", "launch", "accepted", "ready"),
    ("exec: command dispatch", "exec", "accepted", "command_started"),
    ("exec: command execution", "exec", "command_started", "command_finished"),
    ("exec: facade total", "exec", "accepted", "command_finished"),
    ("destroy: cleanup dispatch", "destroy", "accepted", "cleanup_started"),
    ("destroy: cleanup", "destroy", "cleanup_started", "cleanup_finished"),
    ("destroy: facade total", "destroy", "accepted", "cleanup_finished"),
)
HARNESS_STAGE = "harness: process and transport overhead"


def stage_values(
    samples: Sequence[Mapping[str, object]],
) -> dict[str, list[int]]:
    """Return every available stage duration keyed by its published label."""

    values: dict[str, list[int]] = {label: [] for label, *_ in STAGE_DEFINITIONS}
    values[HARNESS_STAGE] = []
    for sample in samples:
        elapsed = _elapsed(sample)
        for label, operation, start, end in STAGE_DEFINITIONS:
            duration = _duration(elapsed.get(operation), start, end)
            if duration is not None:
                values[label].append(duration)
        overhead = _overhead(sample, elapsed)
        if overhead is not None:
            values[HARNESS_STAGE].append(overhead)
    return {label: found for label, found in values.items() if found}


def _elapsed(sample: Mapping[str, object]) -> dict[str, dict[str, int]]:
    stages = sample.get("stages")
    if not isinstance(stages, Mapping):
        return {}
    found: dict[str, dict[str, int]] = {}
    for operation, milestones in stages.items():
        if not isinstance(milestones, list):
            continue
        found[operation] = {
            str(milestone["kind"]): int(milestone["elapsed_ns"])
            for milestone in milestones
            if isinstance(milestone, Mapping)
        }
    return found


def _duration(
    milestones: Mapping[str, int] | None, start: str, end: str
) -> int | None:
    if milestones is None or start not in milestones or end not in milestones:
        return None
    return milestones[end] - milestones[start]


def _overhead(
    sample: Mapping[str, object], elapsed: Mapping[str, Mapping[str, int]]
) -> int | None:
    tti = sample.get("tti_ns")
    launch = _duration(elapsed.get("launch"), "accepted", "ready")
    execute = _duration(elapsed.get("exec"), "accepted", "command_finished")
    if type(tti) is not int or launch is None or execute is None:
        return None
    return tti - launch - execute
