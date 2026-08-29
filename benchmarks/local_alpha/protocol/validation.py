"""Validation of CLI and MCP evidence against one matrix scenario."""

from __future__ import annotations

import base64
import binascii
import json
from collections.abc import Mapping
from typing import Any

from benchmarks.local_alpha.matrix import Scenario

from .model import (
    CliCall,
    McpCall,
    ProtocolValidationError,
    ResponseEvidence,
)


_NETWORK_KEYS = {
    "lease",
    "runtime_attachment",
    "address_leases",
    "egress_policy",
    "dns_policy",
    "proxy_policy",
    "ingress_bindings",
}


def _document(
    value: bytes | str | Mapping[str, Any], label: str
) -> Mapping[str, Any]:
    if isinstance(value, (bytes, str)):
        try:
            value = json.loads(value)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ProtocolValidationError(f"{label} is not valid JSON") from error
    if not isinstance(value, Mapping):
        raise ProtocolValidationError(f"{label} must be an object")
    return value


def _object(value: object, label: str) -> Mapping[str, Any]:
    if not isinstance(value, Mapping):
        raise ProtocolValidationError(f"{label} must be an object")
    return value


def _stdout(result: Mapping[str, Any], expected: str | None) -> str:
    output = _object(result.get("stdout"), "stdout")
    data = output.get("data")
    if output.get("encoding") != "base64" or not isinstance(data, str):
        raise ProtocolValidationError("stdout must use base64 encoding")
    try:
        decoded = base64.b64decode(data, validate=True)
    except (ValueError, binascii.Error) as error:
        raise ProtocolValidationError("stdout contains invalid base64") from error
    if output.get("byte_length") != len(decoded):
        raise ProtocolValidationError("stdout byte length is inconsistent")
    observed = decoded.hex()
    if expected is not None and observed != expected.lower():
        raise ProtocolValidationError("stdout bytes do not match the scenario")
    return observed


def _outcome(status: object, mcp: bool) -> str | None:
    if mcp:
        body = status if isinstance(status, Mapping) else {}
        kind = body.get("kind")
        code = body.get("code")
    else:
        exited = status.get("exited") if isinstance(status, Mapping) else None
        kind = "exited" if isinstance(exited, Mapping) else status
        code = exited.get("code") if isinstance(exited, Mapping) else None
    if kind == "exited" and isinstance(code, int) and not isinstance(code, bool):
        return "success" if code == 0 else "nonzero_exit"
    return {
        "timed_out": "timeout",
        "output_limit_exceeded": "output_limit",
    }.get(kind)


def _complete_cleanup(receipt: Mapping[str, Any]) -> None:
    cleanup = receipt.get("cleanup")
    if cleanup == "complete":
        return
    cleanup = _object(cleanup, "receipt cleanup")
    methods = {"graceful", "forced", "graceful_then_forced"}
    if cleanup.get("method") not in methods:
        raise ProtocolValidationError("cleanup method is not terminal")
    for name in ("machine", "memory", "storage", "guest_authority"):
        if cleanup.get(name) != "complete":
            raise ProtocolValidationError(f"cleanup resource {name} is not complete")
    network = _object(cleanup.get("network"), "network cleanup")
    terminal = all(
        value in {"complete", "not_owned"} for value in network.values()
    )
    if set(network) != _NETWORK_KEYS or not terminal:
        raise ProtocolValidationError("network cleanup is not complete")


def _validate(
    envelope: Mapping[str, Any],
    scenario: Scenario,
    operation: str,
    operation_id: str,
    instance_id: str,
    mcp: bool,
) -> ResponseEvidence:
    schema = "soma.mcp.v1" if mcp else "soma.cli.v1"
    if envelope.get("schema") != schema:
        raise ProtocolValidationError(f"response schema must be {schema}")
    expected = operation if mcp else (
        "run" if operation == "run" else f"machine.{operation}"
    )
    if envelope.get("operation" if mcp else "command") != expected:
        raise ProtocolValidationError("response operation does not match")
    if mcp and envelope.get("operation_id") != operation_id:
        raise ProtocolValidationError("response operation identity does not match")
    result = _object(envelope.get("result"), "response result")
    receipt = _object(envelope.get("receipt"), "response receipt")
    if result.get("instance_id") != instance_id:
        raise ProtocolValidationError("response instance identity does not match")
    if (
        receipt.get("instance_id") != instance_id
        or receipt.get("operation_id") != operation_id
    ):
        raise ProtocolValidationError("receipt identity does not match")
    cleanup: bool | None = None
    if operation in {"run", "destroy"}:
        _complete_cleanup(receipt)
        cleanup = True
    if operation not in {"run", "exec"}:
        expected_state = "ready" if operation == "launch" else "destroyed"
        if result.get("state") != expected_state:
            raise ProtocolValidationError("managed state does not match")
        return ResponseEvidence(operation, instance_id, None, None, cleanup)
    status_key = "status" if mcp else "execution"
    observed = _outcome(result.get(status_key), mcp)
    if observed != scenario.workload.expected_outcome:
        raise ProtocolValidationError("workload outcome does not match the scenario")
    stdout = _stdout(result, scenario.workload.expected_stdout_hex)
    return ResponseEvidence(operation, instance_id, observed, stdout, cleanup)


def validate_cli_response(
    response: bytes | str | Mapping[str, Any],
    *,
    scenario: Scenario,
    call: CliCall,
    instance_id: str,
) -> ResponseEvidence:
    envelope = _document(response, "CLI response")
    successful = (
        call.operation not in {"run", "exec"}
        or scenario.workload.expected_outcome == "success"
    )
    if envelope.get("status") != ("ok" if successful else "error"):
        raise ProtocolValidationError("CLI response status does not match")
    return _validate(
        envelope,
        scenario,
        call.operation,
        call.operation_id,
        instance_id,
        False,
    )


def validate_mcp_response(
    response: bytes | str | Mapping[str, Any],
    *,
    scenario: Scenario,
    call: McpCall,
    instance_id: str,
) -> ResponseEvidence:
    document = _document(response, "MCP response")
    if document.get("schema") == "soma.mcp.v1":
        envelope = document
    else:
        if document.get("error") is not None:
            raise ProtocolValidationError("MCP returned a JSON-RPC error")
        result = _object(document.get("result"), "MCP call result")
        envelope = _object(
            result.get("structuredContent"), "MCP structured content"
        )
    return _validate(
        envelope,
        scenario,
        call.operation,
        call.operation_id,
        instance_id,
        True,
    )
