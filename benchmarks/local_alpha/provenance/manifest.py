"""Strict external build-manifest serialization and release validation."""

from __future__ import annotations

import json
import os
import subprocess
from collections.abc import Sequence
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

from .fingerprint import benchmark_fingerprint, file_sha256, source_fingerprint


MANIFEST_SCHEMA = "soma.local-alpha.build.v2"
MAXIMUM_MANIFEST_BYTES = 64 * 1024
RELEASE_BUILD_COMMAND = (
    "cargo",
    "build",
    "--locked",
    "--release",
    "-p",
    "soma-cli",
    "-p",
    "soma-mcp",
)


def _fixed_hex(value: object, lengths: set[int]) -> bool:
    return (
        isinstance(value, str)
        and len(value) in lengths
        and set(value) != {"0"}
        and all(character in "0123456789abcdef" for character in value)
    )


def _utc_timestamp(value: object) -> bool:
    if not isinstance(value, str):
        return False
    try:
        parsed = datetime.fromisoformat(value)
    except ValueError:
        return False
    return parsed.tzinfo is not None and parsed.utcoffset() == UTC.utcoffset(parsed)


def _unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, nested in pairs:
        if key in value:
            raise ValueError(f"build manifest contains duplicate key: {key}")
        value[key] = nested
    return value


def _git_metadata(root: Path) -> tuple[str, bool]:
    top_level = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    revision = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    status = subprocess.run(
        ["git", "status", "--porcelain"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if (
        top_level.returncode != 0
        or revision.returncode != 0
        or status.returncode != 0
        or Path(top_level.stdout.strip()).resolve() != root.resolve()
    ):
        raise ValueError("build manifest requires an exact Git checkout")
    return revision.stdout.strip(), not bool(status.stdout)


@dataclass(frozen=True, slots=True)
class BinaryIdentity:
    placeholder: str
    filename: str
    sha256: str
    size_bytes: int

    @classmethod
    def create(cls, path: Path, placeholder: str) -> "BinaryIdentity":
        return cls(placeholder, path.name, file_sha256(path), path.stat().st_size)

    def as_dict(self) -> dict[str, object]:
        return {
            "path": self.placeholder,
            "filename": self.filename,
            "sha256": self.sha256,
            "size_bytes": self.size_bytes,
        }

    @classmethod
    def from_dict(
        cls, value: object, placeholder: str, filename: str
    ) -> "BinaryIdentity":
        keys = {"path", "filename", "sha256", "size_bytes"}
        if not isinstance(value, dict) or set(value) != keys:
            raise ValueError("binary identity has an invalid shape")
        size = value["size_bytes"]
        if (
            value["path"] != placeholder
            or value["filename"] != filename
            or not _fixed_hex(value["sha256"], {64})
            or type(size) is not int
            or size <= 0
        ):
            raise ValueError("binary identity is invalid")
        return cls(placeholder, filename, value["sha256"], size)


@dataclass(frozen=True, slots=True)
class BuildManifest:
    schema: str
    created_at_utc: str
    source_sha256: str
    benchmark_sha256: str
    git_revision: str
    worktree_clean: bool
    build_argv: tuple[str, ...]
    soma: BinaryIdentity
    soma_mcp: BinaryIdentity

    @classmethod
    def create(
        cls,
        root: Path,
        soma_binary: Path,
        mcp_binary: Path,
        build_argv: Sequence[str],
        *,
        git_revision: str | None = None,
        worktree_clean: bool | None = None,
    ) -> "BuildManifest":
        """Create a manifest during the external build step."""

        if git_revision is None or worktree_clean is None:
            observed_revision, observed_clean = _git_metadata(root)
            git_revision = git_revision or observed_revision
            worktree_clean = observed_clean if worktree_clean is None else worktree_clean
        manifest = cls(
            MANIFEST_SCHEMA,
            datetime.now(UTC).isoformat(),
            source_fingerprint(root),
            benchmark_fingerprint(root),
            git_revision,
            worktree_clean,
            tuple(build_argv),
            BinaryIdentity.create(soma_binary, "$SOMA_BIN"),
            BinaryIdentity.create(mcp_binary, "$SOMA_MCP_BIN"),
        )
        return cls.from_dict(manifest.as_dict())

    def as_dict(self) -> dict[str, object]:
        return {
            "schema": self.schema,
            "created_at_utc": self.created_at_utc,
            "source_sha256": self.source_sha256,
            "benchmark_sha256": self.benchmark_sha256,
            "git_revision": self.git_revision,
            "worktree_clean": self.worktree_clean,
            "build_argv": list(self.build_argv),
            "binaries": {"soma": self.soma.as_dict(), "soma_mcp": self.soma_mcp.as_dict()},
        }

    @classmethod
    def from_dict(cls, value: object) -> "BuildManifest":
        keys = {
            "schema", "created_at_utc", "source_sha256", "benchmark_sha256",
            "git_revision", "worktree_clean", "build_argv", "binaries",
        }
        if not isinstance(value, dict) or set(value) != keys:
            raise ValueError("build manifest has an invalid shape")
        binaries, argv = value["binaries"], value["build_argv"]
        valid_argv = type(argv) is list and tuple(argv) == RELEASE_BUILD_COMMAND
        if not isinstance(binaries, dict) or set(binaries) != {"soma", "soma_mcp"}:
            raise ValueError("build manifest binaries are invalid")
        if (
            value["schema"] != MANIFEST_SCHEMA
            or not _utc_timestamp(value["created_at_utc"])
            or not _fixed_hex(value["source_sha256"], {64})
            or not _fixed_hex(value["benchmark_sha256"], {64})
            or not _fixed_hex(value["git_revision"], {40, 64})
            or type(value["worktree_clean"]) is not bool
            or not valid_argv
        ):
            raise ValueError("build manifest fields are invalid")
        return cls(
            schema=value["schema"],
            created_at_utc=value["created_at_utc"],
            source_sha256=value["source_sha256"],
            benchmark_sha256=value["benchmark_sha256"],
            git_revision=value["git_revision"],
            worktree_clean=value["worktree_clean"],
            build_argv=tuple(argv),
            soma=BinaryIdentity.from_dict(binaries["soma"], "$SOMA_BIN", "soma"),
            soma_mcp=BinaryIdentity.from_dict(
                binaries["soma_mcp"], "$SOMA_MCP_BIN", "soma-mcp"
            ),
        )

    @classmethod
    def load(cls, path: Path) -> "BuildManifest":
        if (
            path.is_symlink()
            or not path.is_file()
            or path.stat().st_size > MAXIMUM_MANIFEST_BYTES
        ):
            raise ValueError("build manifest must be a bounded regular nonsymlink file")
        try:
            value = json.loads(path.read_bytes(), object_pairs_hook=_unique_object)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ValueError("build manifest is not valid JSON") from error
        return cls.from_dict(value)

    def write(self, path: Path) -> None:
        checked = type(self).from_dict(self.as_dict())
        encoded = (
            json.dumps(checked.as_dict(), sort_keys=True, separators=(",", ":")) + "\n"
        ).encode("utf-8")
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(encoded)
            stream.flush()
            os.fsync(stream.fileno())


def _validate_binary_path(path: Path, expected_name: str) -> None:
    if path.is_symlink() or not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f"{expected_name} release binary is invalid")
    if path.name not in {expected_name, f"{expected_name}.exe"} or path.parent.name != "release":
        raise ValueError(f"{expected_name} must be the explicit target/release binary")


def validate_release_build(
    root: Path,
    manifest: BuildManifest,
    soma_binary: Path,
    mcp_binary: Path,
) -> None:
    """Validate current sources and binaries against an externally loaded manifest."""

    checked = BuildManifest.from_dict(manifest.as_dict())
    _validate_binary_path(soma_binary, "soma")
    _validate_binary_path(mcp_binary, "soma-mcp")
    if not checked.worktree_clean:
        raise ValueError("build manifest does not claim a clean build checkout")
    if source_fingerprint(root) != checked.source_sha256:
        raise ValueError("Cargo source changed after the benchmark release build")
    if benchmark_fingerprint(root) != checked.benchmark_sha256:
        raise ValueError("benchmark harness changed after the benchmark release build")
    if file_sha256(soma_binary) != checked.soma.sha256:
        raise ValueError("soma release binary does not match the build manifest")
    if file_sha256(mcp_binary) != checked.soma_mcp.sha256:
        raise ValueError("soma-mcp release binary does not match the build manifest")
