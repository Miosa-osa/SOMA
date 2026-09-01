"""Command-line entry point for burst measurement and report generation."""

from __future__ import annotations

import argparse
import json
import os
import sys
import tempfile
from collections.abc import Sequence
from functools import partial
from pathlib import Path

from benchmarks.local_alpha.provenance import (
    BuildManifest,
    build_child_environment,
    engine_setting_provenance,
    engine_settings,
    validate_release_build,
)
from benchmarks.local_alpha.runner.identities import IdentityGenerator

from . import metadata as host_metadata
from .attribution import breakdown_lines
from benchmarks.local_alpha.mcp_stdio import McpStdioSession
from .mcp_slot import execute_mcp_slot
from .plan import BACKENDS, EXPERIMENT_CLASSES, BurstPlan
from .report import generate
from .run import run_burst


ROOT = Path(__file__).resolve().parents[3]
KVM_STATE_PARENT = Path("/tmp")
LINUX_UNIX_SOCKET_PATH_BYTES = 108
MACHINE_SOCKET_NAME_BYTES = len("/machines/") + 32 + len(".sock") + 1


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(
        prog="python -m benchmarks.local_alpha.burst",
        description="Measure burst time to first command and publish its evidence.",
    )
    commands = result.add_subparsers(dest="command", required=True)
    _add_run(commands.add_parser("run", help="Run one declared burst cohort."))
    _add_report(
        commands.add_parser("report", help="Generate an evidence document.")
    )
    return result


def _add_run(command: argparse.ArgumentParser) -> None:
    command.add_argument("--experiment-class", choices=EXPERIMENT_CLASSES, required=True)
    command.add_argument("--prepared", action="append", default=[])
    command.add_argument("--backend", choices=BACKENDS, required=True)
    command.add_argument("--image", required=True)
    command.add_argument("--iterations", type=int, required=True)
    command.add_argument("--concurrency", type=int, required=True)
    command.add_argument("--vcpus", type=int, default=1)
    command.add_argument("--memory-mib", type=int, default=1024)
    command.add_argument("--storage-mib", type=int, default=10_240)
    command.add_argument("--timeout-ms", type=int, default=30_000)
    command.add_argument("--max-output-bytes", type=int, default=1_048_576)
    command.add_argument("--build-manifest", type=Path, required=True)
    command.add_argument("--soma-bin", type=Path, required=True)
    command.add_argument("--soma-mcp-bin", type=Path, required=True)
    command.add_argument("--transport", choices=("cli", "mcp"), default="cli")
    command.add_argument("--results", type=Path, required=True)
    command.add_argument("workload", nargs=argparse.REMAINDER)


def _add_report(command: argparse.ArgumentParser) -> None:
    command.add_argument("--results", type=Path, action="append", required=True)
    command.add_argument("--title", required=True)
    command.add_argument("--output", type=Path, required=True)


def main(argv: Sequence[str] | None = None) -> int:
    argument_parser = parser()
    arguments = argument_parser.parse_args(argv)
    try:
        if arguments.command == "report":
            return _report(arguments)
        return _run(arguments)
    except ValueError as error:
        # Not `argparse.error`: it buries one sentence under a usage block, and a caller that
        # only reads the results file sees a run that produced nothing and said nothing.
        sys.stderr.write(f"soma-burst: error: {error}\n")
        sys.stderr.flush()
        raise SystemExit(2) from error


def _run(arguments: argparse.Namespace) -> int:
    workload = tuple(
        arguments.workload[1:]
        if arguments.workload and arguments.workload[0] == "--"
        else arguments.workload
    )
    plan = BurstPlan.create(
        experiment_class=arguments.experiment_class,
        prepared_before_timer=tuple(arguments.prepared),
        backend=arguments.backend,
        image=arguments.image,
        command=workload,
        vcpus=arguments.vcpus,
        memory_mib=arguments.memory_mib,
        storage_mib=arguments.storage_mib,
        iterations=arguments.iterations,
        concurrency=arguments.concurrency,
        timeout_ms=arguments.timeout_ms,
        max_output_bytes=arguments.max_output_bytes,
    )
    manifest_path = _absolute(arguments.build_manifest, "build manifest")
    soma = _release_binary(arguments.soma_bin, "soma")
    mcp = _release_binary(arguments.soma_mcp_bin, "soma-mcp")
    results = _new_file(arguments.results, "results file")
    manifest = BuildManifest.load(manifest_path)
    validate_release_build(ROOT, manifest, soma, mcp)
    settings = engine_settings(os.environ) if plan.backend == "kvm" else {}
    environment = build_child_environment(os.environ, settings)
    state_parent = KVM_STATE_PARENT if plan.backend == "kvm" else results.parent
    with tempfile.TemporaryDirectory(prefix="soma-burst-", dir=state_parent) as temporary:
        state_root = Path(temporary)
        if plan.backend == "kvm":
            _require_addressable_kvm_state(state_root)
        collected = host_metadata.collect(
            plan,
            run_id=IdentityGenerator().new(),
            manifest=manifest,
            soma_binary=soma,
            state_root=state_root,
            environment=environment,
            engine=engine_setting_provenance(settings),
        )
        collected["transport"] = arguments.transport
        if arguments.transport == "mcp":
            mcp_argv = (os.fspath(mcp), "--state-root", os.fspath(state_root))
            with McpStdioSession(
                mcp_argv,
                display_argv=("$SOMA_MCP_BIN", "--state-root", "$STATE_ROOT"),
                environment=environment,
                response_timeout_seconds=300.0,
            ) as client:
                client.initialize("2025-06-18")
                summary = run_burst(
                    plan,
                    soma_binary=soma,
                    state_root=state_root,
                    environment=environment,
                    metadata=collected,
                    results_path=results,
                    slot=partial(execute_mcp_slot, client=client),
                )
        else:
            summary = run_burst(
                plan,
                soma_binary=soma,
                state_root=state_root,
                environment=environment,
                metadata=collected,
                results_path=results,
            )
    json.dump(summary, sys.stdout, sort_keys=True, separators=(",", ":"))
    sys.stdout.write("\n")
    for note in summary.get("shape_disagreements") or []:
        sys.stderr.write(f"soma-burst: {note}\n")
    accepted = summary["tti"]["accepted_count"]
    if accepted == summary["attempted"]:
        sys.stderr.flush()
        return 0
    _report_failures(summary, accepted)
    return 1


def _report_failures(summary: dict[str, object], accepted: int) -> None:
    """Say on stderr why a run did not score what it attempted.

    A score alone is not a result. Every reason is already retained per slot, so the only thing
    that made a zero unreadable was that nothing summarised it where a person would look.
    """

    attempted = summary["attempted"]
    sys.stderr.write(
        f"soma-burst: {accepted} of {attempted} samples succeeded. Why:\n"
    )
    lines = breakdown_lines(summary.get("failure_breakdown") or [])
    if not lines:
        lines = ["no failure reason was retained; this is a harness fault"]
    for line in lines:
        sys.stderr.write(f"  {line}\n")
    sys.stderr.flush()


def _report(arguments: argparse.Namespace) -> int:
    paths = [_existing_file(path, "results file") for path in arguments.results]
    output = _new_file(arguments.output, "output document")
    document = generate(paths, title=arguments.title)
    descriptor = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(document.encode("utf-8"))
    sys.stdout.write(f"{output.name}\n")
    return 0


def _absolute(path: Path, label: str) -> Path:
    if not path.is_absolute():
        raise ValueError(
            f"{label} path must be absolute, but {str(path)!r} is relative; "
            f"pass {Path.cwd() / path}"
        )
    return path


def _existing_file(path: Path, label: str) -> Path:
    _absolute(path, label)
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{label} must be an existing nonsymlink file")
    return path


def _new_file(path: Path, label: str) -> Path:
    _absolute(path, label)
    if os.path.lexists(path):
        raise ValueError(f"{label} must not already exist")
    if not path.parent.is_dir():
        raise ValueError(f"{label} parent directory must exist")
    return path


def _release_binary(path: Path, expected_name: str) -> Path:
    _absolute(path, expected_name)
    if path.is_symlink() or not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f"{expected_name} must be a nonsymlink executable file")
    if path.name != expected_name or path.parent.name != "release":
        raise ValueError(f"{expected_name} must be an explicit target/release binary")
    return path


def _require_addressable_kvm_state(state_root: Path) -> None:
    encoded = len(os.fsencode(state_root)) + MACHINE_SOCKET_NAME_BYTES
    if encoded > LINUX_UNIX_SOCKET_PATH_BYTES:
        raise ValueError(
            "temporary KVM state root cannot address managed machine sockets"
        )
