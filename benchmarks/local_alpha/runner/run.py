"""Top-level benchmark orchestration and artifact finalization."""

from __future__ import annotations

import os
import tempfile
from collections.abc import Callable
from pathlib import Path
from typing import Any

from benchmarks.local_alpha.artifacts import (
    RAW_SCHEMA,
    SUMMARY_SCHEMA,
    ArtifactWriter,
    validate_artifact_directory,
)
from benchmarks.local_alpha.mcp_stdio import McpStdioSession
from benchmarks.local_alpha.provenance import (
    BinaryIdentity,
    BuildManifest,
    build_child_environment,
    validate_release_build,
)

from .cli_sample import execute_cli_sample
from .config import RunnerConfig
from .identities import IdentityGenerator
from .mcp_sample import execute_mcp_sample
from .model import SampleOutcome
from .summary import (
    CompactSample,
    metric_summaries,
    summary_preparation_class,
)


MCP_PROTOCOL_VERSION = "2024-11-05"


def run_benchmark(
    config: RunnerConfig,
    *,
    identities: IdentityGenerator | None = None,
    cli_sampler: Callable[..., SampleOutcome] = execute_cli_sample,
    mcp_sampler: Callable[..., SampleOutcome] = execute_mcp_sample,
    session_factory: Callable[..., Any] = McpStdioSession,
    build_manifest: BuildManifest,
) -> dict[str, object]:
    manifest = build_manifest
    validate_release_build(
        config.root,
        manifest,
        config.soma_binary,
        config.mcp_binary,
    )
    runtime = BinaryIdentity.create(config.apple_runtime, "$APPLE_RUNTIME_BIN")
    environment = build_child_environment(os.environ)
    identities = identities or IdentityGenerator()
    run_id = identities.new()
    samples: list[CompactSample] = []
    preparation_classes: set[str] = set()

    with tempfile.TemporaryDirectory(prefix="soma-local-alpha-state-") as temporary:
        state_root = Path(temporary)
        with ArtifactWriter(config.result_directory) as writer:
            if config.scenario.caller == "cli":
                writer.append(_metadata(config, run_id, manifest, runtime, None))
                for _ in range(config.repetitions):
                    sample = _append_sample(
                        writer,
                        config,
                        run_id,
                        cli_sampler(
                            config.scenario,
                            soma_binary=config.soma_binary,
                            apple_runtime=config.apple_runtime,
                            state_root=state_root,
                            environment=environment,
                            identities=identities,
                        ),
                        len(samples) + 1,
                    )
                    samples.append(sample)
                    preparation_classes.update(sample.preparation_classes)
                    if len(preparation_classes) > 1:
                        raise ValueError("observed receipt preparation classes differ")
                    if not sample.cleanup_validated:
                        break
            else:
                argv = (
                    os.fspath(config.mcp_binary),
                    "--runtime",
                    os.fspath(config.apple_runtime),
                    "--state-root",
                    os.fspath(state_root),
                )
                display_argv = (
                    "$SOMA_MCP_BIN",
                    "--runtime",
                    "$APPLE_RUNTIME_BIN",
                    "--state-root",
                    "$STATE_ROOT",
                )
                with session_factory(
                    argv,
                    display_argv=display_argv,
                    environment=environment,
                    response_timeout_seconds=180.0,
                ) as session:
                    initialization = session.initialize(MCP_PROTOCOL_VERSION)
                    writer.append(
                        _metadata(config, run_id, manifest, runtime, initialization.as_dict())
                    )
                    for _ in range(config.repetitions):
                        sample = _append_sample(
                            writer,
                            config,
                            run_id,
                            mcp_sampler(
                                config.scenario,
                                session=session,
                                identities=identities,
                            ),
                            len(samples) + 1,
                        )
                        samples.append(sample)
                        preparation_classes.update(sample.preparation_classes)
                        if len(preparation_classes) > 1:
                            raise ValueError("observed receipt preparation classes differ")
                        if not sample.cleanup_validated:
                            break
                writer.append(_mcp_process_record(run_id, session))

            summary = _summary(config, run_id, samples)
            writer.finish(summary)

    validate_artifact_directory(config.result_directory)
    return summary


def _append_sample(
    writer: ArtifactWriter,
    config: RunnerConfig,
    run_id: str,
    outcome: SampleOutcome,
    repetition: int,
) -> CompactSample:
    writer.append(
        outcome.as_record(
            run_id=run_id,
            sample_id=f"{run_id}-{repetition:06d}",
            scenario_id=config.scenario.identifier,
            repetition=repetition,
        )
    )
    return CompactSample.from_outcome(outcome)


def _metadata(
    config: RunnerConfig,
    run_id: str,
    manifest: BuildManifest,
    runtime: BinaryIdentity,
    initialization: dict[str, object] | None,
) -> dict[str, object]:
    excluded = ["release build", "temporary state-root creation"]
    if config.scenario.mode == "managed":
        excluded.append("destroy")
    if config.scenario.caller == "mcp":
        excluded.append("MCP process start and initialization")
    document: dict[str, object] = {
        "schema": RAW_SCHEMA,
        "record_type": "run_metadata",
        "run_id": run_id,
        "scenario": config.scenario.as_dict(),
        "repetitions": config.repetitions,
        "concurrency": 1,
        "cache_state": "cached",
        "cache_state_source": "caller_supplied",
        "evidence_class": "apple_virtualization_development",
        "build_performed_by_runner": False,
        "build_manifest": manifest.as_dict(),
        "apple_runtime": runtime.as_dict(),
        "state_root": "$STATE_ROOT",
        "child_environment_keys": sorted(build_child_environment(os.environ)),
        "excluded_work": excluded,
    }
    if initialization is not None:
        document["mcp_process_argv"] = [
            "$SOMA_MCP_BIN",
            "--runtime",
            "$APPLE_RUNTIME_BIN",
            "--state-root",
            "$STATE_ROOT",
        ]
        document["mcp_initialization"] = initialization
    return document


def _mcp_process_record(run_id: str, session: Any) -> dict[str, object]:
    return {
        "schema": RAW_SCHEMA,
        "record_type": "mcp_process",
        "run_id": run_id,
        "argv": list(session.display_argv),
        "exit_code": session.exit_code,
        "stderr": session.stderr_capture.as_dict(),
    }


def _summary(
    config: RunnerConfig, run_id: str, samples: list[CompactSample]
) -> dict[str, object]:
    return {
        "schema": SUMMARY_SCHEMA,
        "run_id": run_id,
        "scenario": config.scenario.as_dict(),
        "repetitions": config.repetitions,
        "attempted_repetitions": len(samples),
        "concurrency": 1,
        "cache_state": "cached",
        "cache_state_source": "caller_supplied",
        "evidence_class": "apple_virtualization_development",
        "preparation_class": summary_preparation_class(samples),
        "external_tti_boundary": samples[0].boundary,
        "metrics": metric_summaries(samples),
        "cleanup_validated_count": sum(item.cleanup_validated for item in samples),
        "all_samples_accepted": (
            len(samples) == config.repetitions and all(item.accepted for item in samples)
        ),
    }
