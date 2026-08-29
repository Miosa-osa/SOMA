"""Scenario translation into explicit SOMA MCP tool calls."""

from __future__ import annotations

from collections.abc import Mapping

from benchmarks.local_alpha.matrix import Scenario

from .common import identity, network, operation_ids as checked_operation_ids
from .model import McpCall


def build_mcp_calls(
    scenario: Scenario,
    *,
    instance_id: str,
    operation_ids: Mapping[str, str],
) -> tuple[McpCall, ...]:
    """Build one-shot or launch/exec/destroy MCP tool calls."""

    if scenario.caller != "mcp":
        raise ValueError("MCP calls require an MCP scenario")
    instance_id = identity(instance_id, "instance ID")
    ids = checked_operation_ids(scenario, operation_ids)
    _, network_input = network(scenario)
    workload = scenario.workload
    machine = {
        "instance_id": instance_id,
        "image": scenario.image,
        "vcpu_count": scenario.shape.vcpus,
        "memory_mib": scenario.shape.memory_mib,
        "storage_mib": scenario.shape.storage_mib,
        "network": network_input,
        "backend": "macos",
    }
    command = {
        "executable": workload.executable,
        "arguments": list(workload.arguments),
        "timeout_ms": workload.timeout_ms,
        "max_output_bytes": workload.maximum_output_bytes,
    }

    def call(operation: str, arguments: Mapping[str, object]) -> McpCall:
        body = {"operation_id": ids[operation], **arguments}
        return McpCall(operation, ids[operation], f"soma_{operation}", body)

    if scenario.mode == "one_shot":
        return (call("run", {**machine, **command}),)
    return (
        call("launch", machine),
        call(
            "exec",
            {"instance_id": instance_id, "backend": "macos", **command},
        ),
        call("destroy", {"instance_id": instance_id, "backend": "macos"}),
    )
