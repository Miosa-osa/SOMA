"""Fail-closed reading of the soma.cli.v1 envelope for one burst slot."""

from __future__ import annotations

from collections.abc import Mapping

from benchmarks.local_alpha.protocol import cleanup_is_complete


_TERMINAL_KINDS = frozenset(
    {"exited", "signaled", "timed_out", "output_limit_exceeded"}
)


def launched_ready(envelope: Mapping[str, object], instance_id: str) -> bool:
    """Report whether a launch envelope proves a ready machine for this instance."""

    result = _result(envelope, "machine.launch", instance_id)
    return result is not None and result.get("state") == "ready"


def destroyed(envelope: Mapping[str, object] | None, instance_id: str) -> bool:
    """Report whether a destroy envelope proves terminal release of the instance."""

    if envelope is None:
        return False
    result = _result(envelope, "machine.destroy", instance_id)
    receipt = envelope.get("receipt")
    return (
        result is not None
        and result.get("state") == "destroyed"
        and isinstance(receipt, Mapping)
        and cleanup_is_complete(receipt)
    )


def command_evidence(
    envelope: Mapping[str, object], instance_id: str
) -> dict[str, object] | None:
    """Return the guest command status and its exact output, or None when invalid."""

    result = _result(envelope, "machine.exec", instance_id)
    if result is None:
        return None
    execution = result.get("execution")
    if isinstance(execution, Mapping) and set(execution) == {"exited"}:
        exited = execution["exited"]
        if not isinstance(exited, Mapping) or type(exited.get("code")) is not int:
            return None
        status, exit_code = "exited", exited["code"]
    elif isinstance(execution, str) and execution in _TERMINAL_KINDS:
        status, exit_code = execution, None
    else:
        return None
    stdout = _stream(result.get("stdout"))
    stderr = _stream(result.get("stderr"))
    if stdout is None or stderr is None:
        return None
    return {
        "status": status,
        "exit_code": exit_code,
        "stdout": stdout,
        "stderr": stderr,
    }


def _result(
    envelope: Mapping[str, object], command: str, instance_id: str
) -> Mapping[str, object] | None:
    result = envelope.get("result")
    if (
        envelope.get("schema") != "soma.cli.v1"
        or envelope.get("status") != "ok"
        or envelope.get("command") != command
        or not isinstance(result, Mapping)
        or result.get("instance_id") != instance_id
    ):
        return None
    return result


def _stream(value: object) -> dict[str, object] | None:
    if (
        not isinstance(value, Mapping)
        or value.get("encoding") != "base64"
        or type(value.get("byte_length")) is not int
        or not isinstance(value.get("data"), str)
    ):
        return None
    return {
        "encoding": "base64",
        "byte_length": value["byte_length"],
        "data_base64": value["data"],
    }
