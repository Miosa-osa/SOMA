"""One persistent-MCP burst slot with launch-through-first-command timing."""

from __future__ import annotations

import time
from collections.abc import Callable, Mapping

from benchmarks.local_alpha.mcp_stdio import McpStdioSession
from benchmarks.local_alpha.protocol import cleanup_is_complete
from .plan import NETWORK_POLICY, BurstPlan
from .slot import BurstSample, _failure, _judge_command, _record_receipt

MCP_BOUNDARY = (
    "immediately_before_the_persistent_mcp_soma_launch_request_to_immediately_after_"
    "the_soma_exec_structured_response; includes_json_rpc_write_read_and_parsing;_"
    "excludes_mcp_process_start_initialization_and_destroy"
)


def execute_mcp_slot(
    plan: BurstPlan,
    *,
    client: McpStdioSession,
    instance_id: str,
    operation_ids: Mapping[str, str],
    clock: Callable[[], int] = time.perf_counter_ns,
    **_: object,
) -> BurstSample:
    """Run one slot through an already initialized MCP session."""

    processes: dict[str, object] = {}
    stages: dict[str, object] = {}
    observed: dict[str, object] = {}
    details: dict[str, str] = {}
    failures: list[dict[str, str]] = []

    def invoke(
        operation: str, arguments: Mapping[str, object]
    ) -> tuple[bool, Mapping[str, object] | None]:
        started = clock()
        try:
            capture = client.call_tool(f"soma_{operation}", arguments)
        except (OSError, RuntimeError, TimeoutError) as error:
            processes[operation] = {
                "protocol_error": type(error).__name__,
                "duration_ns": clock() - started,
            }
            details[operation] = type(error).__name__
            return False, None
        processes[operation] = {
            "duration_ns": capture.duration_ns,
            "clock": capture.clock,
        }
        result = capture.response.get("result")
        if not isinstance(result, Mapping):
            details[operation] = "missing_result"
            return False, None
        envelope = result.get("structuredContent")
        if not isinstance(envelope, Mapping) or result.get("isError") is True:
            details[operation] = "tool_error"
            return False, envelope if isinstance(envelope, Mapping) else None
        _record_receipt(stages, observed, operation, envelope)
        return True, envelope

    common = {"instance_id": instance_id, "backend": plan.backend}
    started_ns = clock()
    launch_ran, launched = invoke(
        "launch",
        {
            **common,
            "operation_id": operation_ids["launch"],
            "image": plan.image,
            "vcpu_count": plan.vcpus,
            "memory_mib": plan.memory_mib,
            "storage_mib": plan.storage_mib,
            "network": {"egress": NETWORK_POLICY, "dns": NETWORK_POLICY},
        },
    )
    if not launch_ran:
        failures.append(_failure("launch_process_failed", "launch", details["launch"]))
        launched = None
    elif not _mcp_machine_state(launched, "launch", instance_id, "ready"):
        failures.append(_failure("launch_response_invalid", "launch"))
        launched = None

    command = None
    tti_ns = None
    command_succeeded = False
    if launched is not None:
        exec_ran, executed = invoke(
            "exec",
            {
                **common,
                "operation_id": operation_ids["exec"],
                "executable": plan.command[0],
                "arguments": list(plan.command[1:]),
                "timeout_ms": plan.timeout_ms,
                "max_output_bytes": plan.max_output_bytes,
            },
        )
        tti_ns = clock() - started_ns
        compatible = _cli_compatible_exec(executed) if executed is not None else None
        command_succeeded, command = _judge_command(
            exec_ran, compatible, instance_id, failures, details
        )

    destroy_ran, destroy = invoke(
        "destroy", {**common, "operation_id": operation_ids["destroy"]}
    )
    cleanup_complete = (
        destroy_ran
        and _mcp_machine_state(destroy, "destroy", instance_id, "destroyed")
        and _cleanup_complete(destroy)
    )
    if not cleanup_complete:
        failures.append(_failure("cleanup_failed", "destroy", details.get("destroy", "")))
    return BurstSample(
        instance_id=instance_id,
        operation_ids=dict(operation_ids),
        tti_ns=tti_ns,
        command_succeeded=command_succeeded,
        cleanup_complete=cleanup_complete,
        processes=processes,
        stages=stages,
        observed=observed,
        command=command,
        failures=tuple(failures),
        boundary=MCP_BOUNDARY,
    )


def _mcp_machine_state(
    envelope: Mapping[str, object] | None,
    operation: str,
    instance_id: str,
    state: str,
) -> bool:
    if envelope is None or envelope.get("schema") != "soma.mcp.v1":
        return False
    result = envelope.get("result")
    return (
        envelope.get("operation") == operation
        and isinstance(result, Mapping)
        and result.get("instance_id") == instance_id
        and result.get("state") == state
    )


def _cli_compatible_exec(envelope: Mapping[str, object]) -> Mapping[str, object] | None:
    if envelope.get("schema") != "soma.mcp.v1" or envelope.get("operation") != "exec":
        return None
    result = envelope.get("result")
    if not isinstance(result, Mapping):
        return None
    status = result.get("status")
    if not isinstance(status, Mapping):
        return None
    kind = status.get("kind")
    execution: object = (
        {"exited": {"code": status.get("code")}} if kind == "exited" else kind
    )
    return {
        "schema": "soma.cli.v1",
        "status": "ok",
        "command": "machine.exec",
        "result": {**result, "execution": execution},
    }


def _cleanup_complete(envelope: Mapping[str, object] | None) -> bool:
    receipt = envelope.get("receipt") if envelope else None
    if not isinstance(receipt, Mapping):
        return False
    return cleanup_is_complete(receipt)
