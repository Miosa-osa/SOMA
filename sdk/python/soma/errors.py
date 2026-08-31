"""Errors raised by the SOMA Python SDK.

The hierarchy mirrors the `soma.cli.v1` failure codes so callers can branch on a
type instead of matching a message string. `NotSupportedYet` is deliberately not
a subclass of `SomaCliError`: it is never produced by the CLI, it is produced by
this SDK when the provider contract asks for something the SOMA command line
cannot do yet.
"""

from __future__ import annotations

__all__ = [
    "BackendUnavailable",
    "GuestTimeout",
    "NotSupportedYet",
    "OutputLimitExceeded",
    "ProtocolError",
    "SandboxNotFound",
    "SomaCliError",
    "SomaError",
    "StateConflict",
    "error_for_code",
]


class SomaError(Exception):
    """Base class for every error this SDK raises."""


class NotSupportedYet(SomaError):
    """A contract operation the `soma` binary cannot perform yet.

    Raised instead of emulating the operation. A silent fallback would let a
    caller believe SOMA has a capability it does not have, which is a worse
    failure than an immediate refusal.
    """

    def __init__(self, capability: str, reason: str) -> None:
        super().__init__(f"{capability} is not supported yet: {reason}")
        self.capability = capability
        self.reason = reason


class ProtocolError(SomaError):
    """The CLI produced something that is not a valid `soma.cli.v1` envelope."""


class SomaCliError(SomaError):
    """A failure the CLI reported inside its envelope."""

    def __init__(
        self,
        code: str,
        message: str,
        *,
        retryable: bool = False,
        exit_code: int | None = None,
        receipt: dict[str, object] | None = None,
    ) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message
        self.retryable = retryable
        self.exit_code = exit_code
        self.receipt = receipt


class SandboxNotFound(SomaCliError):
    """The requested instance identity is unknown to the local state store."""


class StateConflict(SomaCliError):
    """Durable sandbox state rejected the operation."""


class GuestTimeout(SomaCliError):
    """The guest command exceeded its deadline."""


class OutputLimitExceeded(SomaCliError):
    """The guest produced more output than the declared allowance."""


class BackendUnavailable(SomaCliError):
    """The selected isolation backend is not usable on this host."""


_CODE_CLASSES: dict[str, type[SomaCliError]] = {
    "machine_not_found": SandboxNotFound,
    "state_conflict": StateConflict,
    "guest_timeout": GuestTimeout,
    "output_limit": OutputLimitExceeded,
    "backend_unavailable": BackendUnavailable,
    "unsupported_backend": BackendUnavailable,
}


def error_for_code(code: str) -> type[SomaCliError]:
    """Return the exception class that represents one CLI failure code."""

    return _CODE_CLASSES.get(code, SomaCliError)
