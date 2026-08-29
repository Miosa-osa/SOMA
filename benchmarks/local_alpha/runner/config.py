"""Validated runner inputs and canonical scenario selection."""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from benchmarks.local_alpha.matrix import Scenario, build_scenario_matrix


def canonical_scenario(identifier: str) -> Scenario:
    matches = tuple(
        scenario
        for scenario in build_scenario_matrix()
        if scenario.identifier == identifier
    )
    if len(matches) != 1:
        raise ValueError(f"unknown canonical scenario: {identifier}")
    return matches[0]


@dataclass(frozen=True, slots=True)
class RunnerConfig:
    root: Path
    scenario: Scenario
    repetitions: int
    soma_binary: Path
    mcp_binary: Path
    apple_runtime: Path
    result_directory: Path
    cache_state: str

    @classmethod
    def create(
        cls,
        *,
        scenario_id: str,
        repetitions: int,
        soma_binary: Path,
        mcp_binary: Path,
        apple_runtime: Path,
        result_directory: Path,
        cache_state: str,
    ) -> "RunnerConfig":
        if type(repetitions) is not int or repetitions <= 0:
            raise ValueError("repetitions must be positive")
        if cache_state != "cached":
            raise ValueError("cache state must be caller-supplied cached")
        _release_binary(soma_binary, "soma")
        _release_binary(mcp_binary, "soma-mcp")
        _executable(apple_runtime, "Apple runtime")
        if apple_runtime.name != "container":
            raise ValueError("Apple runtime executable must be named container")
        if not result_directory.is_absolute():
            raise ValueError("result directory must be an absolute path")
        if os.path.lexists(result_directory):
            raise ValueError("result directory must not already exist")
        return cls(
            root=Path(__file__).resolve().parents[3],
            scenario=canonical_scenario(scenario_id),
            repetitions=repetitions,
            soma_binary=soma_binary,
            mcp_binary=mcp_binary,
            apple_runtime=apple_runtime,
            result_directory=result_directory,
            cache_state=cache_state,
        )


def _release_binary(path: Path, expected_name: str) -> None:
    _executable(path, expected_name)
    if path.name != expected_name or path.parent.name != "release":
        raise ValueError(f"{expected_name} must be an explicit target/release binary")


def _executable(path: Path, label: str) -> None:
    if not path.is_absolute():
        raise ValueError(f"{label} path must be absolute")
    if path.is_symlink() or not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f"{label} path must be a nonsymlink executable file")
