"""Receipt extraction and available monotonic metric calculation."""

from __future__ import annotations

import json
from collections.abc import Mapping
from typing import Any

from benchmarks.local_alpha.metrics import duration_ns, parse_milestones, run_metrics


_BOUNDARIES = {
    "launch": {
        "image_resolve": ("accepted", "workload_resolved"),
        "launch_ready": ("admitted", "ready"),
        "request_total": ("accepted", "ready"),
    },
    "exec": {
        "command": ("command_started", "command_finished"),
        "request_total": ("accepted", "command_finished"),
    },
    "destroy": {
        "cleanup": ("cleanup_started", "cleanup_finished"),
        "request_total": ("accepted", "cleanup_finished"),
    },
}


def cli_receipt(response: bytes) -> Mapping[str, object]:
    document = json.loads(response)
    if not isinstance(document, Mapping) or not isinstance(document.get("receipt"), Mapping):
        raise ValueError("CLI response receipt is unavailable")
    return document["receipt"]


def mcp_receipt(response: Mapping[str, Any]) -> Mapping[str, object]:
    envelope: object = response
    if response.get("schema") != "soma.mcp.v1":
        result = response.get("result")
        envelope = result.get("structuredContent") if isinstance(result, Mapping) else None
    if not isinstance(envelope, Mapping) or not isinstance(envelope.get("receipt"), Mapping):
        raise ValueError("MCP response receipt is unavailable")
    return envelope["receipt"]


def available_metrics(operation: str, receipt: Mapping[str, object]) -> dict[str, int]:
    if "milestones" not in receipt:
        return {}
    parse_milestones(receipt)
    if operation == "run":
        try:
            values = run_metrics(receipt)
        except ValueError:
            values = _available_boundaries(
                receipt,
                {
                    "image_resolve": ("accepted", "workload_resolved"),
                    "launch_ready": ("admitted", "ready"),
                    "command": ("command_started", "command_finished"),
                    "cleanup": ("cleanup_started", "cleanup_finished"),
                    "request_total": ("accepted", "cleanup_finished"),
                },
            )
    else:
        values = _available_boundaries(receipt, _BOUNDARIES.get(operation, {}))
    return {f"{operation}.{name}": value for name, value in values.items()}


def _available_boundaries(
    receipt: Mapping[str, object], boundaries: Mapping[str, tuple[str, str]]
) -> dict[str, int]:
    values: dict[str, int] = {}
    for name, (start, end) in boundaries.items():
        try:
            values[name] = duration_ns(receipt, start, end)
        except ValueError as error:
            if "missing" not in str(error):
                raise
    return values
