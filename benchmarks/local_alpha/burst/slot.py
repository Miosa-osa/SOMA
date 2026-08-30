"""One burst slot: launch, workload command, and the excluded destruction."""

from __future__ import annotations

import json
import time
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from pathlib import Path

from benchmarks.local_alpha.capture import ProcessCapture, run_external_process
from benchmarks.local_alpha.metrics import parse_milestones

from .envelope import command_evidence, destroyed, launched_ready
from .invocation import display_argv, slot_calls
from .plan import BurstPlan


BOUNDARY = (
    "immediately_before_the_launch_process_capture_to_immediately_after_the_exec_"
    "process_exit_and_pipe_drain; includes_two_soma_process_spawns_and_response_"
    "reading; excludes_destroy"
)
FAILURE_REASONS = (
    "launch_process_failed",
    "launch_response_invalid",
    "command_process_failed",
    "command_response_invalid",
    "command_unsuccessful",
    "cleanup_failed",
)
PROCESS_TIMEOUT_SECONDS = 300.0
MAXIMUM_STREAM_BYTES = 8 * 1024 * 1024


@dataclass(frozen=True, slots=True)
class BurstSample:
    """The retained evidence of one attempted time-to-first-command sample."""

    instance_id: str
    operation_ids: dict[str, str]
    tti_ns: int | None
    command_succeeded: bool
    cleanup_complete: bool
    processes: dict[str, object]
    stages: dict[str, object]
    observed: dict[str, object]
    command: dict[str, object] | None
    failures: tuple[dict[str, str], ...]

    @property
    def successful(self) -> bool:
        return self.command_succeeded and self.cleanup_complete and not self.failures

    def as_record(
        self,
        *,
        plan: BurstPlan,
        run_id: str,
        repetition: int,
        burst_index: int,
        slot_index: int,
    ) -> dict[str, object]:
        return {
            "record_type": "sample",
            "run_id": run_id,
            "sample_id": f"{run_id}-{repetition:06d}",
            "experiment_class": plan.experiment_class,
            "repetition": repetition,
            "burst_index": burst_index,
            "slot_index": slot_index,
            "boundary": BOUNDARY,
            "clock": "time.perf_counter_ns",
            "tti_ns": self.tti_ns,
            "successful": self.successful,
            "command_succeeded": self.command_succeeded,
            "cleanup_complete": self.cleanup_complete,
            "instance_id": self.instance_id,
            "operation_ids": dict(self.operation_ids),
            "processes": dict(self.processes),
            "stages": dict(self.stages),
            "observed": dict(self.observed),
            "command": self.command,
            "failures": [dict(failure) for failure in self.failures],
        }


def execute_slot(
    plan: BurstPlan,
    *,
    soma_binary: Path,
    state_root: Path,
    environment: Mapping[str, str],
    instance_id: str,
    operation_ids: Mapping[str, str],
    capture_process: Callable[..., ProcessCapture] = run_external_process,
    clock: Callable[[], int] = time.perf_counter_ns,
) -> BurstSample:
    """Run one slot and retain its evidence whether or not it succeeded."""

    calls = slot_calls(
        plan,
        soma_binary=soma_binary,
        state_root=state_root,
        instance_id=instance_id,
        operation_ids=operation_ids,
    )
    processes: dict[str, object] = {}
    stages: dict[str, object] = {}
    observed: dict[str, object] = {}
    failures: list[dict[str, str]] = []

    def invoke(operation: str) -> tuple[bool, Mapping[str, object] | None]:
        argv = calls[operation]
        try:
            capture = capture_process(
                argv,
                display_argv=display_argv(
                    argv, soma_binary=soma_binary, state_root=state_root
                ),
                environment=environment,
                timeout_seconds=PROCESS_TIMEOUT_SECONDS,
                maximum_stream_bytes=MAXIMUM_STREAM_BYTES,
            )
        except OSError as error:
            processes[operation] = {"spawn_error": type(error).__name__}
            return False, None
        processes[operation] = {
            "exit_code": capture.exit_code,
            "duration_ns": capture.duration_ns,
            "harness_timed_out": capture.harness_timed_out,
            "stderr": capture.stderr.as_dict(),
        }
        if capture.harness_timed_out or capture.exit_code != 0:
            return False, None
        envelope = _envelope(capture.stdout.retained)
        if envelope is not None:
            _record_receipt(stages, observed, operation, envelope)
        return True, envelope

    started_ns = clock()
    launch_ran, launched = invoke("launch")
    if not launch_ran:
        failures.append(_failure("launch_process_failed", "launch"))
    elif launched is None or not launched_ready(launched, instance_id):
        launched = None
        failures.append(_failure("launch_response_invalid", "launch"))

    command: dict[str, object] | None = None
    tti_ns: int | None = None
    command_succeeded = False
    if launched is not None:
        exec_ran, executed = invoke("exec")
        tti_ns = clock() - started_ns
        command_succeeded, command = _judge_command(
            exec_ran, executed, instance_id, failures
        )

    cleanup_complete = destroyed(invoke("destroy")[1], instance_id)
    if not cleanup_complete:
        failures.append(_failure("cleanup_failed", "destroy"))
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
    )


def _judge_command(
    executed_ran: bool,
    executed: Mapping[str, object] | None,
    instance_id: str,
    failures: list[dict[str, str]],
) -> tuple[bool, dict[str, object] | None]:
    if not executed_ran:
        failures.append(_failure("command_process_failed", "exec"))
        return False, None
    command = command_evidence(executed, instance_id) if executed else None
    if command is None:
        failures.append(_failure("command_response_invalid", "exec"))
        return False, None
    if command["status"] != "exited" or command["exit_code"] != 0:
        failures.append(
            _failure(
                "command_unsuccessful",
                "exec",
                f"{command['status']}:{command['exit_code']}",
            )
        )
        return False, command
    return True, command


def _envelope(response: bytes) -> Mapping[str, object] | None:
    try:
        envelope = json.loads(response)
    except (UnicodeDecodeError, json.JSONDecodeError):
        return None
    return envelope if isinstance(envelope, Mapping) else None


def _record_receipt(
    stages: dict[str, object],
    observed: dict[str, object],
    operation: str,
    envelope: Mapping[str, object],
) -> None:
    receipt = envelope.get("receipt")
    if not isinstance(receipt, Mapping):
        return
    if operation == "launch":
        observed.update(
            {
                "backend": receipt.get("backend"),
                "isolation": receipt.get("isolation"),
                "preparation": receipt.get("preparation"),
                "workload": receipt.get("workload"),
                "effective_shape": receipt.get("effective_shape"),
                "effective_network": receipt.get("effective_network"),
            }
        )
    if "milestones" not in receipt:
        return
    try:
        milestones = parse_milestones(receipt)
    except ValueError:
        stages[operation] = "malformed"
        return
    stages[operation] = [
        {"kind": milestone.kind, "elapsed_ns": milestone.elapsed_ns}
        for milestone in milestones
    ]


def _failure(reason: str, operation: str, detail: str = "") -> dict[str, str]:
    if reason not in FAILURE_REASONS:
        raise ValueError("failure reason is not typed")
    return {"reason": reason, "operation": operation, "detail": detail}
