"""Why a slot failed, in words the run's own output can carry.

A run that scores zero has to say why without anyone reading per slot records by hand. Every
`soma` process the harness spawns already reports an exit code, and every refusal it prints
already carries a typed error code, so the reason exists; it was only being discarded. These
helpers keep it.
"""

from __future__ import annotations

import base64
from collections.abc import Mapping, Sequence


MAXIMUM_DETAIL_CHARACTERS = 160
MAXIMUM_REPORTED_DETAILS = 3

# The command line's semantic exit vocabulary, from crates/soma-cli/src/exit.rs. A number alone
# says nothing; the name it stands for is the difference between "it failed" and "the capability
# is not there".
EXIT_MEANINGS = {
    0: "success",
    2: "usage",
    10: "guest_nonzero",
    65: "invalid_input",
    66: "not_found",
    69: "conflict",
    70: "software",
    73: "output_limit",
    74: "backend_failure",
    75: "cleanup_uncertain",
    76: "capability_unavailable",
    77: "doctor_strict",
    78: "unsupported_backend",
    124: "guest_timeout",
}


def process_detail(
    process: Mapping[str, object], envelope: Mapping[str, object] | None
) -> str:
    """One short attributable reason for a `soma` process that did not succeed."""

    spawn_error = process.get("spawn_error")
    if spawn_error is not None:
        return _bounded(f"spawn_error={spawn_error}")
    parts: list[str] = []
    if process.get("harness_timed_out"):
        parts.append("harness_timed_out")
    exit_code = process.get("exit_code")
    meaning = EXIT_MEANINGS.get(exit_code) if isinstance(exit_code, int) else None
    parts.append(f"exit={exit_code}" + (f"({meaning})" if meaning else ""))
    parts.extend(_envelope_parts(envelope))
    if len(parts) == 1:
        parts.extend(_stderr_parts(process))
    return _bounded(" ".join(parts))


def _envelope_parts(envelope: Mapping[str, object] | None) -> list[str]:
    error = envelope.get("error") if isinstance(envelope, Mapping) else None
    if not isinstance(error, Mapping):
        return []
    parts = [f"code={error.get('code')}"]
    if error.get("retryable") is True:
        parts.append("retryable")
    message = error.get("message")
    if isinstance(message, str) and message:
        parts.append(f"message={message}")
    return parts


def _stderr_parts(process: Mapping[str, object]) -> list[str]:
    stderr = process.get("stderr")
    if not isinstance(stderr, Mapping):
        return []
    encoded = stderr.get("data_base64")
    if isinstance(encoded, str) and encoded:
        try:
            text = base64.b64decode(encoded, validate=True).decode("utf-8", "replace")
        except ValueError:
            return ["stderr=undecodable"]
        if text.strip():
            return [f"stderr={text.strip().splitlines()[0]}"]
    # An empty stderr is itself the finding: the process refused and explained nothing.
    if stderr.get("retained_bytes") == 0 and stderr.get("observed_bytes") == 0:
        return ["stderr=empty"]
    return []


def _bounded(detail: str) -> str:
    collapsed = " ".join(detail.split())
    if len(collapsed) <= MAXIMUM_DETAIL_CHARACTERS:
        return collapsed
    return collapsed[: MAXIMUM_DETAIL_CHARACTERS - 3] + "..."


def failure_breakdown(
    failures: Sequence[Sequence[Mapping[str, str]]],
) -> list[dict[str, object]]:
    """Count every retained failure by reason and keep the details it reported.

    Ordered by count so the first row of a zero scoring run is the thing to fix.
    """

    counts: dict[tuple[str, str], int] = {}
    details: dict[tuple[str, str], dict[str, int]] = {}
    for sample in failures:
        for failure in sample:
            key = (str(failure.get("reason")), str(failure.get("operation")))
            counts[key] = counts.get(key, 0) + 1
            detail = str(failure.get("detail") or "")
            if detail:
                seen = details.setdefault(key, {})
                seen[detail] = seen.get(detail, 0) + 1
    rows = []
    for (reason, operation), count in sorted(
        counts.items(), key=lambda item: (-item[1], item[0])
    ):
        seen = details.get((reason, operation), {})
        rows.append(
            {
                "reason": reason,
                "operation": operation,
                "count": count,
                "details": [
                    {"detail": detail, "count": times}
                    for detail, times in sorted(
                        seen.items(), key=lambda item: (-item[1], item[0])
                    )[:MAXIMUM_REPORTED_DETAILS]
                ],
            }
        )
    return rows


def breakdown_lines(breakdown: Sequence[Mapping[str, object]]) -> list[str]:
    """The breakdown as lines a person reads without opening the results file."""

    lines = []
    for row in breakdown:
        lines.append(
            f"{row['count']}x {row['reason']} at {row['operation']}"
        )
        for detail in row["details"]:
            lines.append(f"    {detail['count']}x {detail['detail']}")
    return lines


SHAPE_DIMENSIONS = ("vcpu_count", "memory_mib", "storage_mib")


def shape_disagreement(observed: Mapping[str, object]) -> str:
    """How the shape a launch delivered differs from the shape it was asked for.

    A launch reports `ok` whether every dimension was observed to match, was observed to differ,
    or was never checked. The third is the dangerous one: a request for ten gigabytes of writable
    storage served by a two gigabyte overlay reports `not_verified` and looks like a success.
    """

    requested = observed.get("requested_shape")
    effective = observed.get("effective_shape")
    if not isinstance(requested, Mapping) or not isinstance(effective, Mapping):
        return ""
    notes = []
    for name in SHAPE_DIMENSIONS:
        asked = requested.get(name)
        given = effective.get(name)
        if not isinstance(given, Mapping):
            continue
        if given.get("state") != "observed":
            notes.append(f"{name} requested {asked}, effective {given.get('value')}")
        elif given.get("value") != asked:
            notes.append(f"{name} requested {asked}, effective {given.get('value')}")
    if not notes:
        return ""
    return "shape mismatch: " + "; ".join(notes)
