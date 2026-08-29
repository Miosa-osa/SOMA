"""Public protocol-plan and validation result types."""

from __future__ import annotations

from dataclasses import dataclass


MACOS_DNS_SERVER = "1.1.1.1"


class ProtocolValidationError(ValueError):
    """A SOMA response did not prove the scenario's expected result."""


@dataclass(frozen=True, slots=True)
class CliCall:
    operation: str
    operation_id: str
    argv: tuple[str, ...]


@dataclass(frozen=True, slots=True)
class McpCall:
    operation: str
    operation_id: str
    tool_name: str
    arguments: dict[str, object]


@dataclass(frozen=True, slots=True)
class ResponseEvidence:
    operation: str
    instance_id: str
    outcome: str | None
    stdout_hex: str | None
    cleanup_complete: bool | None
