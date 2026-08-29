"""Controlled release build for local-alpha benchmark execution."""

from __future__ import annotations

import argparse
import os
import subprocess
from collections.abc import Callable, Sequence
from pathlib import Path

from benchmarks.local_alpha.provenance import (
    RELEASE_BUILD_COMMAND,
    BuildManifest,
)
from benchmarks.local_alpha.provenance.manifest import _git_metadata


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
CommandRunner = Callable[..., subprocess.CompletedProcess[object]]
CheckoutProbe = Callable[[Path], tuple[str, bool]]


def _remove_previous_output(path: Path) -> None:
    if not os.path.lexists(path):
        return
    if path.is_dir() and not path.is_symlink():
        raise ValueError(f"release output path is a directory: {path.name}")
    path.unlink()


def _require_fresh_output(path: Path) -> None:
    if (
        path.is_symlink()
        or not path.is_file()
        or not os.access(path, os.X_OK)
        or path.stat().st_size <= 0
    ):
        raise ValueError(f"Cargo did not create a fresh release output: {path.name}")


def build_release(
    root: Path,
    manifest_path: Path,
    *,
    run_command: CommandRunner = subprocess.run,
    checkout_probe: CheckoutProbe = _git_metadata,
) -> BuildManifest:
    """Build the two benchmark binaries and exclusively write their manifest."""

    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        raise ValueError("repository root must be an absolute nonsymlink directory")
    if not manifest_path.is_absolute():
        raise ValueError("build manifest destination must be absolute")
    resolved_root = root.resolve()
    resolved_manifest = manifest_path.resolve()
    if resolved_manifest == resolved_root or resolved_root in resolved_manifest.parents:
        raise ValueError("build manifest destination must be outside the source checkout")
    if os.path.lexists(manifest_path):
        raise ValueError("build manifest destination must not already exist")
    if not manifest_path.parent.is_dir():
        raise ValueError("build manifest parent directory must already exist")

    revision, clean = checkout_probe(root)
    if not clean:
        raise ValueError("release build requires a clean Git checkout")

    release = root / "target" / "release"
    soma_binary = release / "soma"
    mcp_binary = release / "soma-mcp"
    for output in (soma_binary, mcp_binary):
        _remove_previous_output(output)

    completed = run_command(RELEASE_BUILD_COMMAND, cwd=root, check=True)
    if completed.returncode != 0:
        raise subprocess.CalledProcessError(
            completed.returncode, RELEASE_BUILD_COMMAND
        )

    final_revision, final_clean = checkout_probe(root)
    if not final_clean or final_revision != revision:
        raise ValueError("Git checkout changed during the release build")
    for output in (soma_binary, mcp_binary):
        _require_fresh_output(output)

    manifest = BuildManifest.create(
        root,
        soma_binary,
        mcp_binary,
        RELEASE_BUILD_COMMAND,
        git_revision=final_revision,
        worktree_clean=True,
    )
    manifest.write(manifest_path)
    return manifest


def _parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(
        prog="python -m benchmarks.local_alpha.build_release",
        description="Build benchmark release binaries and write a v2 manifest.",
    )
    result.add_argument("--build-manifest", type=Path, required=True)
    return result


def main(argv: Sequence[str] | None = None) -> int:
    arguments = _parser().parse_args(argv)
    build_release(REPOSITORY_ROOT, arguments.build_manifest)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
