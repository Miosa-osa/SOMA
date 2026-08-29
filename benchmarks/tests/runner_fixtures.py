"""Shared fixtures for local-alpha runner behavior tests."""

from __future__ import annotations

import os
from pathlib import Path

from benchmarks.local_alpha.provenance import BuildManifest
from benchmarks.local_alpha.runner.command import parse_arguments
from benchmarks.local_alpha.runner.model import SampleOutcome


def make_config(root: Path, *, caller: str = "cli", repetitions: int = 2):
    release = root / "target" / "release"
    release.mkdir(parents=True)
    runtime = root / "container"
    for executable in (release / "soma", release / "soma-mcp", runtime):
        executable.write_bytes(b"fixture")
        executable.chmod(0o700)
    return parse_arguments(
        [
            "--scenario-id",
            f"base-{caller}-one-shot-node-22-1vcpu-1024mib-10240mib-denied",
            "--repetitions",
            str(repetitions),
            "--soma-bin",
            os.fspath(release / "soma"),
            "--soma-mcp-bin",
            os.fspath(release / "soma-mcp"),
            "--apple-runtime",
            os.fspath(runtime),
            "--result-dir",
            os.fspath(root / "results"),
            "--cache-state",
            "cached",
        ]
    )


def build_manifest(config) -> BuildManifest:
    return BuildManifest.create(
        config.root,
        config.soma_binary,
        config.mcp_binary,
        ["fixture-build"],
        git_revision="a" * 40,
        worktree_clean=True,
    )


def sample_outcome(
    identities,
    *,
    duration_ns: int = 10,
    accepted: bool = True,
    cleanup_validated: bool = True,
) -> SampleOutcome:
    return SampleOutcome(
        instance_id=identities.new(),
        operation_ids={"run": identities.new()},
        duration_ns=duration_ns,
        boundary="fixture-boundary",
        accepted=accepted,
        cleanup_validated=cleanup_validated,
        operations=(),
        receipt_metrics_ns={"run.command": 3} if accepted else {},
        errors=() if accepted else ({"operation": "run", "type": "cleanup"},),
    )
