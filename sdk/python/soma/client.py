"""The provider surface: create, get by id, list, destroy, run a command."""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass

from . import envelope as envelopes
from .envelope import ExecResult
from .errors import NotSupportedYet, ProtocolError
from .process import Cli, Runner
from .sandbox import Sandbox, guest_argv

__all__ = ["Shape", "Soma"]

_GUEST_NONZERO = "guest_nonzero"


@dataclass(frozen=True)
class Shape:
    """The machine shape and network policy for a new sandbox.

    Each field is left unset by default so the CLI's own defaults stay the
    single source of truth: one vCPU, 1024 MiB of memory, 10240 MiB of storage,
    and denied egress and DNS.
    """

    vcpus: int | None = None
    memory_mib: int | None = None
    storage_mib: int | None = None
    egress: str | None = None
    dns: str | None = None
    dns_servers: Sequence[str] = ()
    publish: Sequence[str] = ()

    def flags(self) -> list[str]:
        arguments: list[str] = []
        if self.vcpus is not None:
            arguments += ["--vcpus", str(self.vcpus)]
        if self.memory_mib is not None:
            arguments += ["--memory-mib", str(self.memory_mib)]
        if self.storage_mib is not None:
            arguments += ["--storage-mib", str(self.storage_mib)]
        if self.egress is not None:
            arguments += ["--egress", self.egress]
        if self.dns is not None:
            arguments += ["--dns", self.dns]
        for server in self.dns_servers:
            arguments += ["--dns-server", str(server)]
        for publication in self.publish:
            arguments += ["--publish", str(publication)]
        return arguments


class Soma:
    """A SOMA sandbox provider backed by the local `soma` binary."""

    def __init__(
        self,
        binary: str = "soma",
        *,
        backend: str | None = None,
        state_root: str | None = None,
        runtime: str | None = None,
        runner: Runner | None = None,
    ) -> None:
        self._cli = Cli(
            binary,
            backend=backend,
            state_root=state_root,
            runtime=runtime,
            runner=runner,
        )

    def create(
        self,
        image: str,
        *,
        name: str | None = None,
        shape: Shape | None = None,
        instance_id: str | None = None,
        operation_id: str | None = None,
    ) -> Sandbox:
        """Launch a durable sandbox from an OCI image."""

        arguments = ["machine", "launch"]
        if instance_id is not None:
            arguments += ["--instance-id", instance_id]
        if operation_id is not None:
            arguments += ["--operation-id", operation_id]
        if name is not None:
            arguments += ["--name", name]
        arguments += (shape or Shape()).flags()
        arguments.append(image)
        envelope = self._cli.invoke(arguments)
        return Sandbox(_result_instance_id(envelope.result), self._cli)

    def get_by_id(self, instance_id: str) -> Sandbox:
        """Return a handle to an existing sandbox.

        The sandbox is inspected first, so an unknown identity raises
        `SandboxNotFound` here rather than at the first command.
        """

        sandbox = Sandbox(instance_id, self._cli)
        sandbox.inspect()
        return sandbox

    def list(self) -> list[Sandbox]:
        """Refused: the CLI cannot enumerate sandboxes."""

        raise NotSupportedYet(
            "sandbox.list",
            "the soma command line has no enumeration command; every operation "
            "addresses one sandbox by its exact instance identity",
        )

    def destroy(self, instance_id: str) -> str:
        """Force-destroy a sandbox by identity and return its reported state."""

        return Sandbox(instance_id, self._cli).destroy()

    def run(
        self,
        image: str,
        command: Sequence[str],
        *,
        name: str | None = None,
        shape: Shape | None = None,
        timeout_ms: int | None = None,
        max_output_bytes: int | None = None,
        instance_id: str | None = None,
        operation_id: str | None = None,
    ) -> ExecResult:
        """Run one command in a throwaway sandbox and prove its cleanup.

        This is `soma run`. It has no ComputeSDK counterpart, but it is the
        cheapest correct way to execute a single command, so it is exposed
        rather than reconstructed from create, exec and destroy.
        """

        arguments = ["run"]
        if instance_id is not None:
            arguments += ["--instance-id", instance_id]
        if operation_id is not None:
            arguments += ["--operation-id", operation_id]
        if name is not None:
            arguments += ["--name", name]
        arguments += (shape or Shape()).flags()
        if timeout_ms is not None:
            arguments += ["--timeout-ms", str(timeout_ms)]
        if max_output_bytes is not None:
            arguments += ["--max-output-bytes", str(max_output_bytes)]
        arguments += [image, "--", *guest_argv(command)]
        envelope = self._cli.invoke(arguments, tolerate=(_GUEST_NONZERO,))
        return envelopes.exec_result(envelope)

    def version(self) -> dict[str, object]:
        """Report the command-line contract version and compiled capabilities."""

        return self._cli.invoke(["version"]).result or {}

    def doctor(self, *, strict: bool = False) -> dict[str, object]:
        """Probe the selected backend."""

        arguments = ["doctor", "--strict"] if strict else ["doctor"]
        return self._cli.invoke(arguments).result or {}


def _result_instance_id(result: dict[str, object] | None) -> str:
    if result is None:
        raise ProtocolError("machine.launch returned no result body")
    instance_id = result.get("instance_id")
    if not isinstance(instance_id, str):
        raise ProtocolError("machine.launch returned no instance identity")
    return instance_id
