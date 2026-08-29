"""One MCP sample on a caller-owned persistent stdio session."""

from __future__ import annotations

import time
from collections.abc import Mapping
from typing import Protocol

from benchmarks.local_alpha.matrix import Scenario
from benchmarks.local_alpha.mcp_stdio import McpFrameCapture
from benchmarks.local_alpha.protocol import build_mcp_calls, validate_mcp_response

from .identities import IdentityGenerator
from .model import SampleOutcome
from .receipts import available_metrics, mcp_receipt
from .summary import observed_preparation_class


class McpCaller(Protocol):
    def call_tool(self, name: str, arguments: Mapping[str, object]) -> McpFrameCapture: ...


def execute_mcp_sample(
    scenario: Scenario,
    *,
    session: McpCaller,
    identities: IdentityGenerator,
    clock=time.perf_counter_ns,
) -> SampleOutcome:
    instance_id = identities.new()
    names = ("run",) if scenario.mode == "one_shot" else ("launch", "exec", "destroy")
    operation_ids = {name: identities.new() for name in names}
    calls = build_mcp_calls(
        scenario,
        instance_id=instance_id,
        operation_ids=operation_ids,
    )
    records: list[dict[str, object]] = []
    errors: list[dict[str, str]] = []
    metrics: dict[str, int] = {}
    cleanup_validated = False
    started_ns: int | None = None
    endpoint_ns: int | None = None

    def invoke(
        call, *, start_timing: bool = False, capture_endpoint: bool = False
    ) -> tuple[bool, McpFrameCapture | None]:
        nonlocal cleanup_validated, endpoint_ns, started_ns
        record: dict[str, object] = {"operation": call.operation}
        records.append(record)
        capture: McpFrameCapture | None = None
        try:
            if start_timing:
                started_ns = clock()
            capture = session.call_tool(call.tool_name, call.arguments)
            if capture_endpoint:
                endpoint_ns = clock()
            record["frame"] = capture.as_dict()
            evidence = validate_mcp_response(
                capture.response,
                scenario=scenario,
                call=call,
                instance_id=instance_id,
            )
            record["validated"] = True
            record["outcome"] = evidence.outcome
            receipt = mcp_receipt(capture.response)
            preparation = observed_preparation_class(receipt)
            if preparation is not None:
                record["preparation_class"] = preparation
            metrics.update(available_metrics(call.operation, receipt))
            if call.operation in {"run", "destroy"}:
                cleanup_validated = evidence.cleanup_complete is True
            return True, capture
        except Exception as error:
            record["validated"] = False
            record["error_type"] = type(error).__name__
            errors.append({"operation": call.operation, "type": type(error).__name__})
            return False, capture

    if scenario.mode == "one_shot":
        started = clock()
        _, capture = invoke(calls[0])
        duration = capture.duration_ns if capture is not None else clock() - started
        boundary = "before_run_jsonrpc_write_to_after_correlated_response_parse"
    else:
        try:
            launch_ok, _ = invoke(
                calls[0], start_timing=True, capture_endpoint=True
            )
            if launch_ok:
                invoke(calls[1], capture_endpoint=True)
        finally:
            invoke(calls[2])
        if started_ns is None or endpoint_ns is None:
            raise RuntimeError("managed MCP timing endpoint was not captured")
        duration = endpoint_ns - started_ns
        boundary = (
            "immediately_before_launch_jsonrpc_call_to_immediately_after_exec_"
            "correlated_response_parse; includes_inter_call_launch_validation_"
            "and_harness_work; excludes_exec_validation_and_destroy"
        )

    return SampleOutcome(
        instance_id=instance_id,
        operation_ids=operation_ids,
        duration_ns=duration,
        boundary=boundary,
        accepted=not errors and cleanup_validated,
        cleanup_validated=cleanup_validated,
        operations=tuple(records),
        receipt_metrics_ns=metrics,
        errors=tuple(errors),
    )
