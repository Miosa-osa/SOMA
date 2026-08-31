"""Decoding of the stable `soma.cli.v1` JSON envelope.

Every `soma --format json` invocation prints exactly one envelope on stdout with
the fields `schema`, `command`, `status`, `result`, `error` and `receipt`. This
module turns that document into typed values and nothing else, so the process
plumbing and the provider surface never touch raw JSON.
"""

from __future__ import annotations

import base64
import json
from dataclasses import dataclass, field
from typing import Any

from .errors import ProtocolError

__all__ = [
    "ENVELOPE_SCHEMA",
    "Envelope",
    "ExecResult",
    "Inspection",
    "exec_result",
    "inspection",
    "parse",
]

ENVELOPE_SCHEMA = "soma.cli.v1"

# The guest command reached one of four terminal states. Only `exited` carries an
# exit code; the SDK refuses to invent one for the other three.
_STATUS_EXITED = "exited"
_STATUS_SIGNALED = "signaled"


@dataclass(frozen=True)
class Envelope:
    """One decoded CLI response."""

    schema: str
    command: str
    status: str
    result: dict[str, Any] | None
    error: dict[str, Any] | None
    receipt: dict[str, Any] | None

    @property
    def ok(self) -> bool:
        return self.error is None


@dataclass(frozen=True)
class ExecResult:
    """The outcome of one bounded guest command."""

    instance_id: str
    stdout: bytes
    stderr: bytes
    exit_code: int | None
    signal: int | None
    status: str
    receipt: dict[str, Any] | None = field(default=None, repr=False)

    @property
    def stdout_text(self) -> str:
        return self.stdout.decode("utf-8", errors="replace")

    @property
    def stderr_text(self) -> str:
        return self.stderr.decode("utf-8", errors="replace")

    @property
    def succeeded(self) -> bool:
        return self.exit_code == 0


@dataclass(frozen=True)
class Inspection:
    """The portable state of one durable sandbox."""

    instance_id: str
    state: str
    backend: str


def parse(stdout: bytes) -> Envelope:
    """Decode one envelope, rejecting anything that is not the known schema."""

    try:
        document = json.loads(stdout)
    except json.JSONDecodeError as cause:
        raise ProtocolError("soma did not print a JSON envelope") from cause
    if not isinstance(document, dict):
        raise ProtocolError("the soma envelope must be a JSON object")
    schema = document.get("schema")
    if schema != ENVELOPE_SCHEMA:
        raise ProtocolError(f"unknown envelope schema {schema!r}")
    return Envelope(
        schema=schema,
        command=_string(document, "command"),
        status=_string(document, "status"),
        result=_optional_object(document, "result"),
        error=_optional_object(document, "error"),
        receipt=_optional_object(document, "receipt"),
    )


def exec_result(envelope: Envelope) -> ExecResult:
    """Read a command result body out of an envelope."""

    body = envelope.result
    if body is None:
        raise ProtocolError(f"{envelope.command} returned no command result")
    status, exit_code, signal = _execution(body.get("execution"))
    return ExecResult(
        instance_id=_string(body, "instance_id"),
        stdout=_output_bytes(body.get("stdout")),
        stderr=_output_bytes(body.get("stderr")),
        exit_code=exit_code,
        signal=signal,
        status=status,
        receipt=envelope.receipt,
    )


def inspection(envelope: Envelope) -> Inspection:
    """Read an inspection result body out of an envelope."""

    body = envelope.result
    if body is None:
        raise ProtocolError(f"{envelope.command} returned no inspection result")
    return Inspection(
        instance_id=_string(body, "instance_id"),
        state=_string(body, "state"),
        backend=_string(body, "backend"),
    )


def _execution(execution: Any) -> tuple[str, int | None, int | None]:
    """Decode the externally tagged `CommandStatus` enum."""

    if isinstance(execution, str):
        return execution, None, None
    if isinstance(execution, dict) and len(execution) == 1:
        name, payload = next(iter(execution.items()))
        if name == _STATUS_EXITED and isinstance(payload, dict):
            return name, _int(payload, "code"), None
        if name == _STATUS_SIGNALED and isinstance(payload, dict):
            signal = payload.get("signal")
            return name, None, signal if isinstance(signal, int) else None
    raise ProtocolError("the command result has an unreadable execution status")


def _output_bytes(captured: Any) -> bytes:
    """Decode one base64 output field and verify its declared length."""

    if not isinstance(captured, dict):
        raise ProtocolError("the command result is missing a captured output field")
    if captured.get("encoding") != "base64":
        raise ProtocolError("captured output is not base64 encoded")
    try:
        decoded = base64.b64decode(str(captured.get("data", "")), validate=True)
    except (ValueError, TypeError) as cause:
        raise ProtocolError("captured output is not valid base64") from cause
    declared = captured.get("byte_length")
    if isinstance(declared, int) and declared != len(decoded):
        raise ProtocolError("captured output length does not match its declaration")
    return decoded


def _string(document: dict[str, Any], key: str) -> str:
    value = document.get(key)
    if not isinstance(value, str):
        raise ProtocolError(f"the envelope field {key!r} must be a string")
    return value


def _int(document: dict[str, Any], key: str) -> int:
    value = document.get(key)
    if not isinstance(value, int):
        raise ProtocolError(f"the envelope field {key!r} must be an integer")
    return value


def _optional_object(document: dict[str, Any], key: str) -> dict[str, Any] | None:
    value = document.get(key)
    if value is None:
        return None
    if not isinstance(value, dict):
        raise ProtocolError(f"the envelope field {key!r} must be an object or null")
    return value
