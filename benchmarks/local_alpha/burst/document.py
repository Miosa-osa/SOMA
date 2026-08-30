"""Markdown fragments shared by the burst evidence document."""

from __future__ import annotations

import base64
import hashlib
import json
from collections.abc import Mapping, Sequence
from pathlib import Path

from .results import BurstResults, statistics
from .stages import stage_values


def table(
    header: Sequence[str], rows: Sequence[Sequence[str]], *, numeric_from: int = 1
) -> list[str]:
    """Render one Markdown table whose columns from `numeric_from` are numeric."""

    alignment = ["---"] * numeric_from + ["---:"] * (len(header) - numeric_from)
    lines = ["| " + " | ".join(header) + " |", "|" + "|".join(alignment) + "|"]
    lines.extend("| " + " | ".join(row) + " |" for row in rows)
    return lines


def compact(value: object) -> str:
    """Render a structured metadata value as compact inline JSON."""

    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def generations(cohorts: Sequence[BurstResults]) -> list[dict[str, object]]:
    """Return each distinct workload identity the launch receipts reported."""

    identities = []
    for workload in distinct_observations(cohorts, "workload"):
        identity = workload.get("identity") if isinstance(workload, Mapping) else None
        if isinstance(identity, Mapping) and identity not in identities:
            identities.append(dict(identity))
    return identities


def nanoseconds(value: object) -> str:
    return "not available" if value is None else f"{int(value):,}"


def milliseconds(value: object) -> str:
    if value is None:
        return "not available"
    return f"{int(value):,} ({int(value) / 1_000_000:.2f} ms)"


def seconds(value: object) -> str:
    if value is None:
        return "not available"
    return f"{int(value):,} ({int(value) / 1_000_000_000:.2f} s)"


def cohort_id(results: BurstResults) -> str:
    plan = results.plan
    image = "".join(
        character if character.isalnum() else "-" for character in str(plan["image"])
    )
    return f"{image}-c{plan['concurrency']}-n{plan['iterations']}"


def cohort_rows(cohorts: Sequence[BurstResults]) -> list[list[str]]:
    rows = []
    for cohort in cohorts:
        plan = cohort.plan
        rows.append(
            [
                cohort_id(cohort),
                f"`{plan['image']}`",
                "`" + " ".join(str(part) for part in plan["command"]) + "`",
                str(plan["concurrency"]),
                str(plan["iterations"]),
                f"{len(cohort.successful)} of {plan['iterations']}",
                seconds(cohort.wall_ns),
            ]
        )
    return rows


def timing_rows(cohorts: Sequence[BurstResults]) -> list[list[str]]:
    rows = []
    for cohort in cohorts:
        summary = cohort.tti_statistics()
        rows.append(
            [
                cohort_id(cohort),
                f"{summary['success_rate'] * 100:.1f}%",
                *(
                    milliseconds(summary[name])
                    for name in ("minimum_ns", "p50_ns", "p95_ns", "p99_ns", "maximum_ns")
                ),
            ]
        )
    return rows


def stage_rows(cohort: BurstResults) -> list[list[str]]:
    rows = []
    for label, values in stage_values(cohort.successful).items():
        summary = statistics(values, failed_count=0)
        rows.append(
            [
                label,
                str(len(values)),
                *(
                    nanoseconds(summary[name])
                    for name in ("minimum_ns", "p50_ns", "p95_ns", "p99_ns", "maximum_ns")
                ),
            ]
        )
    return rows


def failure_lines(cohorts: Sequence[BurstResults]) -> list[str]:
    lines: list[str] = []
    for cohort in cohorts:
        for sample in cohort.failed:
            reasons = ", ".join(
                f"{failure['reason']}"
                + (f" ({failure['detail']})" if failure.get("detail") else "")
                for failure in sample["failures"]
            )
            lines.append(
                f"- `{cohort_id(cohort)}` repetition {sample['repetition']} "
                f"(burst {sample['burst_index']}, slot {sample['slot_index']}): {reasons}."
            )
    if not lines:
        total = sum(len(cohort.samples) for cohort in cohorts)
        lines.append(f"No sample failed; every one of the {total} samples succeeded.")
    return lines


def output_lines(cohorts: Sequence[BurstResults]) -> list[str]:
    lines = []
    for cohort in cohorts:
        distinct = {
            (
                sample["command"]["stdout"]["byte_length"],
                sample["command"]["stdout"]["data_base64"],
            )
            for sample in cohort.successful
        }
        for length, encoded in sorted(distinct):
            lines.append(
                f"- `{cohort_id(cohort)}`: {len(cohort.successful)} successful "
                f"commands returned exit status 0 and {length} stdout bytes "
                f"{_rendered(encoded)}."
            )
    return lines


def raw_data_lines(cohorts: Sequence[BurstResults]) -> list[str]:
    lines = []
    for cohort in cohorts:
        content = cohort.path.read_bytes()
        lines.append(
            f"- `{cohort_id(cohort)}`: `{Path(cohort.path).name}`, "
            f"{content.count(10):,} lines, "
            f"{len(content):,} bytes, SHA-256 "
            f"`{hashlib.sha256(content).hexdigest()}`."
        )
    return lines


def distinct_observations(
    cohorts: Sequence[BurstResults], field: str
) -> list[object]:
    """Return every distinct launch-receipt observation of one field."""

    found: list[object] = []
    for cohort in cohorts:
        for sample in cohort.samples:
            observed = sample.get("observed")
            if not isinstance(observed, Mapping) or field not in observed:
                continue
            value = observed[field]
            if value not in found:
                found.append(value)
    return found


def _rendered(encoded: str) -> str:
    decoded = base64.b64decode(encoded, validate=True)
    if not decoded:
        return "(empty)"
    text = decoded.decode("utf-8", errors="replace")
    if len(decoded) <= 256 and text.strip().isprintable():
        return "exactly `" + text.replace("\n", "\\n") + "`"
    return f"with SHA-256 `{hashlib.sha256(decoded).hexdigest()}`"
