"""A handle to one durable SOMA sandbox."""

from __future__ import annotations

from collections.abc import Sequence

from . import envelope as envelopes
from .envelope import ExecResult, Inspection
from .filesystem import Filesystem
from .process import Cli

__all__ = ["Sandbox", "guest_argv"]

# The CLI reports a nonzero guest exit as an envelope error, but a caller asking
# to run a command wants the exit code back, not an exception.
_GUEST_NONZERO = "guest_nonzero"


def guest_argv(command: Sequence[str]) -> list[str]:
    """Validate an argv for the guest.

    A bare string is refused rather than split. The CLI runs a direct command
    beginning with an absolute guest executable, with no shell in between, so
    splitting a string here would quietly change the meaning of quotes.
    """

    if isinstance(command, (str, bytes)):
        raise TypeError(
            "pass the guest command as an argv sequence, for example "
            "['/bin/sh', '-c', 'echo hi']; there is no shell in the guest"
        )
    argv = [str(argument) for argument in command]
    if not argv:
        raise ValueError("the guest command must have at least one argument")
    return argv


class Sandbox:
    """One durable sandbox, addressed by its exact instance identity."""

    def __init__(self, instance_id: str, cli: Cli) -> None:
        self.instance_id = instance_id
        self._cli = cli
        self._filesystem = Filesystem(instance_id)

    @property
    def filesystem(self) -> Filesystem:
        """The filesystem contract surface. Every method on it refuses."""

        return self._filesystem

    def run_command(
        self,
        command: Sequence[str],
        *,
        timeout_ms: int | None = None,
        max_output_bytes: int | None = None,
        operation_id: str | None = None,
    ) -> ExecResult:
        """Run one bounded command inside this sandbox."""

        arguments = ["machine", "exec", "--instance-id", self.instance_id]
        if operation_id is not None:
            arguments += ["--operation-id", operation_id]
        if timeout_ms is not None:
            arguments += ["--timeout-ms", str(timeout_ms)]
        if max_output_bytes is not None:
            arguments += ["--max-output-bytes", str(max_output_bytes)]
        arguments += ["--", *guest_argv(command)]
        envelope = self._cli.invoke(arguments, tolerate=(_GUEST_NONZERO,))
        return envelopes.exec_result(envelope)

    def inspect(self, *, operation_id: str | None = None) -> Inspection:
        """Read the portable state of this sandbox."""

        envelope = self._cli.invoke(self._control("inspect", operation_id))
        return envelopes.inspection(envelope)

    def stop(self, *, operation_id: str | None = None) -> str:
        """Gracefully stop this sandbox and return its reported state."""

        return self._machine_state("stop", operation_id)

    def destroy(self, *, operation_id: str | None = None) -> str:
        """Force-destroy this sandbox and return its reported state."""

        return self._machine_state("destroy", operation_id)

    def _machine_state(self, verb: str, operation_id: str | None) -> str:
        envelope = self._cli.invoke(self._control(verb, operation_id))
        body = envelope.result or {}
        state = body.get("state")
        return state if isinstance(state, str) else "unknown"

    def _control(self, verb: str, operation_id: str | None) -> list[str]:
        arguments = ["machine", verb, "--instance-id", self.instance_id]
        if operation_id is not None:
            arguments += ["--operation-id", operation_id]
        return arguments

    def __repr__(self) -> str:
        return f"Sandbox(instance_id={self.instance_id!r})"
