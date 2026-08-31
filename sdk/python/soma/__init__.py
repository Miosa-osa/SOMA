"""A dependency-free Python SDK for the SOMA sandbox engine.

It drives the `soma` binary's stable `soma.cli.v1` JSON envelope. Operations the
command line cannot perform yet raise `NotSupportedYet` naming the capability;
nothing here is emulated.
"""

from __future__ import annotations

from .client import Shape, Soma
from .envelope import ENVELOPE_SCHEMA, Envelope, ExecResult, Inspection
from .errors import (
    BackendUnavailable,
    GuestTimeout,
    NotSupportedYet,
    OutputLimitExceeded,
    ProtocolError,
    SandboxNotFound,
    SomaCliError,
    SomaError,
    StateConflict,
)
from .filesystem import Filesystem
from .process import Cli, Completed, Runner, subprocess_runner
from .sandbox import Sandbox

__version__ = "0.1.0"

__all__ = [
    "ENVELOPE_SCHEMA",
    "BackendUnavailable",
    "Cli",
    "Completed",
    "Envelope",
    "ExecResult",
    "Filesystem",
    "GuestTimeout",
    "Inspection",
    "NotSupportedYet",
    "OutputLimitExceeded",
    "ProtocolError",
    "Runner",
    "Sandbox",
    "SandboxNotFound",
    "Shape",
    "Soma",
    "SomaCliError",
    "SomaError",
    "StateConflict",
    "__version__",
    "subprocess_runner",
]
