"""The subprocess boundary between this SDK and the `soma` binary.

Everything that talks to the operating system lives here and is reachable
through a single injectable callable, so the tests can drive the whole SDK with
recorded envelopes and no KVM host.
"""

from __future__ import annotations

import subprocess
from collections.abc import Callable, Sequence
from dataclasses import dataclass

from . import envelope as envelopes
from .envelope import Envelope
from .errors import ProtocolError, error_for_code

__all__ = ["Cli", "Completed", "Runner", "subprocess_runner"]


@dataclass(frozen=True)
class Completed:
    """What one `soma` process left behind."""

    exit_code: int
    stdout: bytes
    stderr: bytes


Runner = Callable[[Sequence[str]], Completed]


def subprocess_runner(argv: Sequence[str]) -> Completed:
    """Run `soma` once and capture its streams."""

    finished = subprocess.run(argv, capture_output=True, check=False)
    return Completed(
        exit_code=finished.returncode,
        stdout=finished.stdout,
        stderr=finished.stderr,
    )


class Cli:
    """A configured `soma` command line.

    The global flags are placed immediately after the binary name because the
    CLI accepts them anywhere before the `--` argv separator, and keeping them
    in one place makes every constructed argv readable in a test failure.
    """

    def __init__(
        self,
        binary: str = "soma",
        *,
        backend: str | None = None,
        state_root: str | None = None,
        runtime: str | None = None,
        runner: Runner | None = None,
    ) -> None:
        self.binary = binary
        self.backend = backend
        self.state_root = state_root
        self.runtime = runtime
        self._runner: Runner = runner or subprocess_runner

    def argv(self, arguments: Sequence[str]) -> list[str]:
        """Build the full argv for one operation."""

        line = [self.binary, "--format", "json"]
        if self.backend is not None:
            line += ["--backend", self.backend]
        if self.runtime is not None:
            line += ["--runtime", self.runtime]
        if self.state_root is not None:
            line += ["--state-root", self.state_root]
        line += list(arguments)
        return line

    def invoke(
        self,
        arguments: Sequence[str],
        *,
        tolerate: Sequence[str] = (),
    ) -> Envelope:
        """Run one operation and return its envelope.

        A failure envelope raises, except for the codes in `tolerate`. That
        exemption exists for `guest_nonzero`, which the CLI reports as an error
        but which is an ordinary result for a caller running a command.
        """

        completed = self._runner(self.argv(arguments))
        if not completed.stdout.strip():
            detail = completed.stderr.decode("utf-8", errors="replace").strip()
            raise ProtocolError(
                f"soma exited {completed.exit_code} without an envelope: {detail}"
            )
        envelope = envelopes.parse(completed.stdout)
        failure = envelope.error
        if failure is None:
            return envelope
        code = str(failure.get("code", "unknown"))
        if code in tolerate:
            return envelope
        raise error_for_code(code)(
            code,
            str(failure.get("message", "")),
            retryable=bool(failure.get("retryable", False)),
            exit_code=completed.exit_code,
            receipt=envelope.receipt,
        )
