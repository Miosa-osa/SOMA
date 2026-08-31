"""A fake `soma` binary at the subprocess boundary.

The envelopes below are shaped exactly like the ones the real CLI prints, so
these tests exercise the whole SDK without KVM, an OCI registry or a host that
can start a virtual machine.
"""

from __future__ import annotations

import base64
import json
from collections.abc import Sequence
from typing import Any

from soma.process import Completed

INSTANCE_ID = "6cba047e2973434f8ebf6380ee8bb72a"


def envelope(
    command: str,
    *,
    result: dict[str, Any] | None = None,
    error: dict[str, Any] | None = None,
    receipt: dict[str, Any] | None = None,
) -> bytes:
    """Build one `soma.cli.v1` envelope exactly as the CLI serializes it."""

    document = {
        "schema": "soma.cli.v1",
        "command": command,
        "status": "error" if error else "ok",
        "result": result,
        "error": error,
        "receipt": receipt,
    }
    return json.dumps(document).encode("utf-8") + b"\n"


def captured(payload: bytes) -> dict[str, Any]:
    """Build one base64 output field."""

    return {
        "encoding": "base64",
        "byte_length": len(payload),
        "data": base64.b64encode(payload).decode("ascii"),
    }


def command_result(
    *,
    stdout: bytes = b"",
    stderr: bytes = b"",
    exit_code: int = 0,
    instance_id: str = INSTANCE_ID,
) -> dict[str, Any]:
    return {
        "instance_id": instance_id,
        "execution": {"exited": {"code": exit_code}},
        "stdout": captured(stdout),
        "stderr": captured(stderr),
    }


def failure(code: str, message: str, *, retryable: bool = False) -> dict[str, Any]:
    return {"code": code, "message": message, "retryable": retryable}


class FakeCli:
    """A runner that replays scripted responses and records every argv."""

    def __init__(self, responses: Sequence[Completed]) -> None:
        self.responses = list(responses)
        self.calls: list[list[str]] = []

    def __call__(self, argv: Sequence[str]) -> Completed:
        self.calls.append(list(argv))
        if not self.responses:
            raise AssertionError(f"the fake CLI had no response for {list(argv)}")
        return self.responses.pop(0)

    @property
    def last_call(self) -> list[str]:
        return self.calls[-1]


def ok(command: str, result: dict[str, Any] | None) -> Completed:
    return Completed(exit_code=0, stdout=envelope(command, result=result), stderr=b"")


def failed(
    command: str,
    code: str,
    message: str,
    *,
    exit_code: int,
    retryable: bool = False,
    result: dict[str, Any] | None = None,
) -> Completed:
    return Completed(
        exit_code=exit_code,
        stdout=envelope(
            command,
            result=result,
            error=failure(code, message, retryable=retryable),
        ),
        stderr=b"",
    )
