"""Canonical local-alpha scenario matrix.

This module contains no runner behavior.
It makes coverage reviewable before any sandbox is started.
"""

from __future__ import annotations

from dataclasses import dataclass
from itertools import product


@dataclass(frozen=True, slots=True)
class Shape:
    vcpus: int
    memory_mib: int
    storage_mib: int = 10_240

    def __post_init__(self) -> None:
        if self.vcpus <= 0 or self.memory_mib <= 0 or self.storage_mib <= 0:
            raise ValueError("shape dimensions must be nonzero")

    @property
    def slug(self) -> str:
        return f"{self.vcpus}vcpu-{self.memory_mib}mib-{self.storage_mib}mib"


@dataclass(frozen=True, slots=True)
class Workload:
    name: str
    executable: str
    arguments: tuple[str, ...]
    expected_outcome: str
    timeout_ms: int = 30_000
    maximum_output_bytes: int = 1_048_576
    expected_stdout_hex: str | None = None

    def __post_init__(self) -> None:
        if not self.executable.startswith("/"):
            raise ValueError("workload executable must be an absolute guest path")
        if self.timeout_ms <= 0 or self.maximum_output_bytes <= 0:
            raise ValueError("workload limits must be nonzero")


@dataclass(frozen=True, slots=True)
class Scenario:
    identifier: str
    kind: str
    caller: str
    mode: str
    image: str
    shape: Shape
    network_policy: str
    workload: Workload

    def __post_init__(self) -> None:
        if self.caller not in {"cli", "mcp"}:
            raise ValueError("unknown caller")
        if self.mode not in {"one_shot", "managed"}:
            raise ValueError("unknown lifecycle mode")
        if self.network_policy not in set(BASE_NETWORK_POLICIES):
            raise ValueError("unknown network policy")
        if not self.identifier.replace("-", "").isalnum():
            raise ValueError("scenario identifier is not path safe")

    def as_dict(self) -> dict[str, object]:
        return {
            "id": self.identifier,
            "kind": self.kind,
            "caller": self.caller,
            "mode": self.mode,
            "image": self.image,
            "shape": {
                "vcpus": self.shape.vcpus,
                "memory_mib": self.shape.memory_mib,
                "storage_mib": self.shape.storage_mib,
            },
            "network_policy": self.network_policy,
            "workload": {
                "name": self.workload.name,
                "executable": self.workload.executable,
                "arguments": list(self.workload.arguments),
                "expected_outcome": self.workload.expected_outcome,
                "timeout_ms": self.workload.timeout_ms,
                "maximum_output_bytes": self.workload.maximum_output_bytes,
                "expected_stdout_hex": self.workload.expected_stdout_hex,
            },
        }


@dataclass(frozen=True, slots=True)
class BurstCohort:
    identifier: str
    caller: str
    mode: str
    image: str
    width: int
    scenario: Scenario


BASE_IMAGES = ("node:22", "ubuntu:24.04")
BASE_SHAPES = (Shape(1, 1_024), Shape(2, 2_048))
BASE_NETWORK_POLICIES = ("unspecified", "denied", "allowed")
MAXIMUM_BURST_WIDTH = 32


def _slug(value: str) -> str:
    return "".join(character if character.isalnum() else "-" for character in value)


def _ready_workload(image: str) -> Workload:
    if image == "node:22":
        return Workload(
            name="ready_command",
            executable="/usr/local/bin/node",
            arguments=("--eval", "process.stdout.write('soma-ready')"),
            expected_outcome="success",
            expected_stdout_hex=b"soma-ready".hex(),
        )
    return Workload(
        name="ready_command",
        executable="/bin/sh",
        arguments=("-c", "printf soma-ready"),
        expected_outcome="success",
        expected_stdout_hex=b"soma-ready".hex(),
    )


def _adverse_workloads() -> tuple[Workload, ...]:
    return (
        Workload(
            name="nonzero_exit",
            executable="/bin/sh",
            arguments=("-c", "exit 17"),
            expected_outcome="nonzero_exit",
        ),
        Workload(
            name="timeout",
            executable="/bin/sh",
            arguments=("-c", "sleep 5"),
            expected_outcome="timeout",
            timeout_ms=100,
        ),
        Workload(
            name="output_limit",
            executable="/bin/sh",
            arguments=("-c", "head -c 4096 /dev/zero"),
            expected_outcome="output_limit",
            maximum_output_bytes=64,
        ),
        Workload(
            name="binary_output",
            executable="/bin/sh",
            arguments=("-c", "printf '\\377\\000\\376\\012'"),
            expected_outcome="success",
            expected_stdout_hex="ff00fe0a",
        ),
    )


def build_scenario_matrix() -> tuple[Scenario, ...]:
    scenarios: list[Scenario] = []
    for caller, mode, image, shape, network in product(
        ("cli", "mcp"),
        ("one_shot", "managed"),
        BASE_IMAGES,
        BASE_SHAPES,
        BASE_NETWORK_POLICIES,
    ):
        identifier = "-".join(
            (
                "base",
                caller,
                mode.replace("_", "-"),
                _slug(image),
                shape.slug,
                network,
            )
        )
        scenarios.append(
            Scenario(
                identifier=identifier,
                kind="base",
                caller=caller,
                mode=mode,
                image=image,
                shape=shape,
                network_policy=network,
                workload=_ready_workload(image),
            )
        )

    for caller, workload in product(("cli", "mcp"), _adverse_workloads()):
        scenarios.append(
            Scenario(
                identifier=f"adverse-{caller}-{workload.name.replace('_', '-')}",
                kind="adverse",
                caller=caller,
                mode="one_shot",
                image="ubuntu:24.04",
                shape=BASE_SHAPES[0],
                network_policy="unspecified",
                workload=workload,
            )
        )

    identifiers = [scenario.identifier for scenario in scenarios]
    if len(identifiers) != len(set(identifiers)):
        raise AssertionError("scenario matrix produced duplicate identifiers")
    return tuple(scenarios)


def build_burst_cohorts(widths: tuple[int, ...]) -> tuple[BurstCohort, ...]:
    if not widths:
        raise ValueError("at least one burst width is required")
    if any(width <= 0 or width > MAXIMUM_BURST_WIDTH for width in widths):
        raise ValueError(f"burst width must be between 1 and {MAXIMUM_BURST_WIDTH}")

    cohorts: list[BurstCohort] = []
    for caller, mode, image, width in product(
        ("cli", "mcp"),
        ("one_shot", "managed"),
        BASE_IMAGES,
        widths,
    ):
        scenario = Scenario(
            identifier=f"burst-member-{caller}-{mode.replace('_', '-')}-{_slug(image)}",
            kind="burst",
            caller=caller,
            mode=mode,
            image=image,
            shape=BASE_SHAPES[0],
            network_policy="unspecified",
            workload=_ready_workload(image),
        )
        cohorts.append(
            BurstCohort(
                identifier=f"burst-{caller}-{mode.replace('_', '-')}-{_slug(image)}-w{width}",
                caller=caller,
                mode=mode,
                image=image,
                width=width,
                scenario=scenario,
            )
        )
    return tuple(cohorts)
