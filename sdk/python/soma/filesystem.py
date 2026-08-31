"""The filesystem half of the provider contract, which the CLI cannot serve yet.

Every method here refuses. The guest control protocol in `soma-guest` already
carries all six operations as `FileRequest`, but the `soma` command line exposes
no way to reach them: there is no file subcommand and no guest path argument
anywhere in `soma run`, `soma machine launch` or `soma machine exec`.

Emulating these calls by shelling out to `cat` and `base64` inside the guest was
considered and rejected. It would work only for images that happen to ship those
tools, it would silently corrupt large or binary payloads at the output limit,
and it would report success for a capability the command line has not exposed. A
refusal that names the missing capability is the honest answer until it does.
"""

from __future__ import annotations

from .errors import NotSupportedYet

__all__ = ["Filesystem"]

_REASON = (
    "the soma command line exposes no file subcommand, although the guest "
    "control protocol already carries the operation; see sdk/README.md"
)


class Filesystem:
    """The contract surface, present so callers fail loudly rather than silently."""

    def __init__(self, instance_id: str) -> None:
        self.instance_id = instance_id

    def read_file(self, path: str) -> bytes:
        raise NotSupportedYet("filesystem.read_file", _REASON)

    def write_file(self, path: str, contents: bytes | str) -> None:
        raise NotSupportedYet("filesystem.write_file", _REASON)

    def mkdir(self, path: str, *, parents: bool = False) -> None:
        raise NotSupportedYet("filesystem.mkdir", _REASON)

    def readdir(self, path: str) -> list[str]:
        raise NotSupportedYet("filesystem.readdir", _REASON)

    def exists(self, path: str) -> bool:
        raise NotSupportedYet("filesystem.exists", _REASON)

    def remove(self, path: str, *, recursive: bool = False) -> None:
        raise NotSupportedYet("filesystem.remove", _REASON)

    def __repr__(self) -> str:
        return f"Filesystem(instance_id={self.instance_id!r}, supported=False)"
