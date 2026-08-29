"""One CLI sample with bounded capture and mandatory managed cleanup."""

from __future__ import annotations

import time
from collections.abc import Callable, Mapping
from pathlib import Path

from benchmarks.local_alpha.capture import ProcessCapture, run_external_process
from benchmarks.local_alpha.matrix import Scenario
from benchmarks.local_alpha.protocol import build_cli_calls, validate_cli_response

from .identities import IdentityGenerator
from .model import SampleOutcome
from .receipts import available_metrics, cli_receipt
from .summary import observed_preparation_class


PROCESS_TIMEOUT_SECONDS = 180.0
MAXIMUM_STREAM_BYTES = 2 * 1024 * 1024
_EXIT_CODES = {"success": 0, "nonzero_exit": 10, "timeout": 124, "output_limit": 73}


def execute_cli_sample(
    scenario: Scenario,
    *,
    soma_binary: Path,
    apple_runtime: Path,
    state_root: Path,
    environment: Mapping[str, str],
    identities: IdentityGenerator,
    capture_process: Callable[..., ProcessCapture] = run_external_process,
    clock: Callable[[], int] = time.perf_counter_ns,
) -> SampleOutcome:
    instance_id = identities.new()
    names = ("run",) if scenario.mode == "one_shot" else ("launch", "exec", "destroy")
    operation_ids = {name: identities.new() for name in names}
    calls = build_cli_calls(
        scenario,
        soma_binary=str(soma_binary),
        instance_id=instance_id,
        operation_ids=operation_ids,
        global_arguments=("--runtime", str(apple_runtime), "--state-root", str(state_root)),
    )
    records: list[dict[str, object]] = []
    errors: list[dict[str, str]] = []
    metrics: dict[str, int] = {}
    cleanup_validated = False
    started_ns: int | None = None
    endpoint_ns: int | None = None

    def invoke(
        call, *, start_timing: bool = False, capture_endpoint: bool = False
    ) -> tuple[bool, ProcessCapture | None]:
        nonlocal cleanup_validated, endpoint_ns, started_ns
        expected = _expected_exit_code(scenario, call.operation)
        display = _display_argv(call.argv, soma_binary, apple_runtime, state_root)
        record: dict[str, object] = {
            "operation": call.operation,
            "expected_process_exit_code": expected,
        }
        records.append(record)
        capture: ProcessCapture | None = None
        try:
            if start_timing:
                started_ns = clock()
            capture = capture_process(
                call.argv,
                display_argv=display,
                environment=environment,
                timeout_seconds=PROCESS_TIMEOUT_SECONDS,
                maximum_stream_bytes=MAXIMUM_STREAM_BYTES,
            )
            if capture_endpoint:
                endpoint_ns = clock()
            record["process"] = capture.as_dict()
            if capture.harness_timed_out or capture.exit_code != expected:
                raise RuntimeError("unexpected CLI process result")
            evidence = validate_cli_response(
                capture.stdout.retained,
                scenario=scenario,
                call=call,
                instance_id=instance_id,
            )
            record["validated"] = True
            record["outcome"] = evidence.outcome
            receipt = cli_receipt(capture.stdout.retained)
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
        boundary = "before_run_process_spawn_to_after_exit_and_pipe_drain"
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
            raise RuntimeError("managed CLI timing endpoint was not captured")
        duration = endpoint_ns - started_ns
        boundary = (
            "immediately_before_launch_process_capture_to_immediately_after_"
            "exec_exit_and_pipe_drain; includes_inter_call_launch_validation_"
            "and_harness_work; excludes_exec_validation_and_destroy"
        )

    accepted = not errors and cleanup_validated
    return SampleOutcome(
        instance_id=instance_id,
        operation_ids=operation_ids,
        duration_ns=duration,
        boundary=boundary,
        accepted=accepted,
        cleanup_validated=cleanup_validated,
        operations=tuple(records),
        receipt_metrics_ns=metrics,
        errors=tuple(errors),
    )


def _expected_exit_code(scenario: Scenario, operation: str) -> int:
    if operation not in {"run", "exec"}:
        return 0
    return _EXIT_CODES[scenario.workload.expected_outcome]


def _display_argv(
    argv: tuple[str, ...], soma: Path, runtime: Path, state_root: Path
) -> tuple[str, ...]:
    replacements = {
        str(soma): "$SOMA_BIN",
        str(runtime): "$APPLE_RUNTIME_BIN",
        str(state_root): "$STATE_ROOT",
    }
    return tuple(replacements.get(value, value) for value in argv)
