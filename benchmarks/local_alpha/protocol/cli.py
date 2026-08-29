"""Scenario translation into explicit SOMA CLI invocations."""

from __future__ import annotations

from collections.abc import Mapping, Sequence

from benchmarks.local_alpha.matrix import Scenario

from .common import identity, network, operation_ids as checked_operation_ids
from .model import CliCall


def _shape(scenario: Scenario) -> tuple[str, ...]:
    value = scenario.shape
    return (
        "--vcpus",
        str(value.vcpus),
        "--memory-mib",
        str(value.memory_mib),
        "--storage-mib",
        str(value.storage_mib),
    )


def _limits(scenario: Scenario) -> tuple[str, ...]:
    value = scenario.workload
    return (
        "--timeout-ms",
        str(value.timeout_ms),
        "--max-output-bytes",
        str(value.maximum_output_bytes),
    )


def build_cli_calls(
    scenario: Scenario,
    *,
    soma_binary: str,
    instance_id: str,
    operation_ids: Mapping[str, str],
    global_arguments: Sequence[str] = (),
) -> tuple[CliCall, ...]:
    """Build one-shot or launch/exec/destroy JSON CLI invocations."""

    if scenario.caller != "cli":
        raise ValueError("CLI calls require a CLI scenario")
    if not soma_binary or any(not isinstance(value, str) for value in global_arguments):
        raise ValueError("CLI process arguments must be strings")
    instance_id = identity(instance_id, "instance ID")
    ids = checked_operation_ids(scenario, operation_ids)
    network_flags, _ = network(scenario)
    base = (
        soma_binary,
        "--format",
        "json",
        "--backend",
        "macos",
        *global_arguments,
    )
    workload = scenario.workload

    def request_identity(operation: str) -> tuple[str, ...]:
        return (
            "--operation-id",
            ids[operation],
            "--instance-id",
            instance_id,
        )

    if scenario.mode == "one_shot":
        argv = (
            *base,
            "run",
            *request_identity("run"),
            *_shape(scenario),
            *network_flags,
            *_limits(scenario),
            scenario.image,
            "--",
            workload.executable,
            *workload.arguments,
        )
        return (CliCall("run", ids["run"], argv),)

    launch = (
        *base,
        "machine",
        "launch",
        *request_identity("launch"),
        *_shape(scenario),
        *network_flags,
        scenario.image,
    )
    execute = (
        *base,
        "machine",
        "exec",
        *request_identity("exec"),
        *_limits(scenario),
        "--",
        workload.executable,
        *workload.arguments,
    )
    destroy = (
        *base,
        "machine",
        "destroy",
        *request_identity("destroy"),
    )
    return (
        CliCall("launch", ids["launch"], launch),
        CliCall("exec", ids["exec"], execute),
        CliCall("destroy", ids["destroy"], destroy),
    )
