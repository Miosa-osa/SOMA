"""Exact soma command lines for one burst slot and their published form."""

from __future__ import annotations

import os
from collections.abc import Mapping
from pathlib import Path

from .plan import NETWORK_POLICY, BurstPlan


OPERATIONS = ("launch", "exec", "destroy")


def slot_calls(
    plan: BurstPlan,
    *,
    soma_binary: Path,
    state_root: Path,
    instance_id: str,
    operation_ids: Mapping[str, str],
) -> dict[str, tuple[str, ...]]:
    """Build the launch, exec, and destroy invocations for one instance."""

    if set(operation_ids) != set(OPERATIONS):
        raise ValueError("one operation identity per lifecycle call is required")
    base = (
        os.fspath(soma_binary),
        "--format",
        "json",
        "--backend",
        plan.backend,
        "--state-root",
        os.fspath(state_root),
    )
    identity = ("--instance-id", instance_id)
    return {
        "launch": (
            *base,
            "machine",
            "launch",
            "--operation-id",
            operation_ids["launch"],
            *identity,
            "--vcpus",
            str(plan.vcpus),
            "--memory-mib",
            str(plan.memory_mib),
            "--storage-mib",
            str(plan.storage_mib),
            "--egress",
            NETWORK_POLICY,
            "--dns",
            NETWORK_POLICY,
            plan.image,
        ),
        "exec": (
            *base,
            "machine",
            "exec",
            "--operation-id",
            operation_ids["exec"],
            *identity,
            "--timeout-ms",
            str(plan.timeout_ms),
            "--max-output-bytes",
            str(plan.max_output_bytes),
            "--",
            *plan.command,
        ),
        "destroy": (
            *base,
            "machine",
            "destroy",
            "--operation-id",
            operation_ids["destroy"],
            *identity,
        ),
    }


def display_argv(
    argv: tuple[str, ...], *, soma_binary: Path, state_root: Path
) -> tuple[str, ...]:
    """Replace host-specific paths with stable placeholders for publication."""

    replacements = {
        os.fspath(soma_binary): "$SOMA_BIN",
        os.fspath(state_root): "$STATE_ROOT",
    }
    return tuple(replacements.get(value, value) for value in argv)
