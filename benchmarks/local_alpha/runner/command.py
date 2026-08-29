"""Command-line parsing for one explicit local-alpha scenario."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Sequence

from benchmarks.local_alpha.provenance import BuildManifest

from .config import RunnerConfig
from .run import run_benchmark


def _positive(value: str) -> int:
    try:
        parsed = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("must be an integer") from error
    if parsed <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return parsed


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(
        prog="python -m benchmarks.local_alpha",
        description="Run one canonical Apple local-alpha benchmark scenario.",
    )
    result.add_argument("--scenario-id", action="append", required=True)
    result.add_argument("--repetitions", type=_positive, required=True)
    result.add_argument("--build-manifest", type=Path, required=True)
    result.add_argument("--soma-bin", "--soma-binary", type=Path, required=True)
    result.add_argument(
        "--soma-mcp-bin", "--soma-mcp-binary", type=Path, required=True
    )
    result.add_argument("--apple-runtime", type=Path, required=True)
    result.add_argument("--result-dir", "--results-dir", type=Path, required=True)
    result.add_argument("--cache-state", choices=("cached",), required=True)
    return result


def parse_arguments(argv: Sequence[str] | None = None) -> RunnerConfig:
    argument_parser = parser()
    arguments = argument_parser.parse_args(argv)
    if len(arguments.scenario_id) != 1:
        argument_parser.error("--scenario-id must be supplied exactly once")
    try:
        return RunnerConfig.create(
            scenario_id=arguments.scenario_id[0],
            repetitions=arguments.repetitions,
            build_manifest=arguments.build_manifest,
            soma_binary=arguments.soma_bin,
            mcp_binary=arguments.soma_mcp_bin,
            apple_runtime=arguments.apple_runtime,
            result_directory=arguments.result_dir,
            cache_state=arguments.cache_state,
        )
    except ValueError as error:
        argument_parser.error(str(error))
    raise AssertionError("argparse.error does not return")


def main(argv: Sequence[str] | None = None) -> int:
    config = parse_arguments(argv)
    manifest = BuildManifest.load(config.build_manifest)
    summary = run_benchmark(config, build_manifest=manifest)
    json.dump(summary, sys.stdout, sort_keys=True, separators=(",", ":"))
    sys.stdout.write("\n")
    return 0 if summary["all_samples_accepted"] is True else 1
