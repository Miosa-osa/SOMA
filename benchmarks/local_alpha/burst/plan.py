"""The declared burst experiment and the contract rules it must satisfy."""

from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import dataclass


EXPERIMENT_CLASSES = (
    "cold-generation-build",
    "cold-cache-restore",
    "warm-cache-restore",
    "prepared-worker",
    "paused-pool",
    "ready-pool",
)
BACKENDS = ("docker", "kvm", "macos")
_CACHE_STATES = {
    "cold-generation-build": "uncached_registry",
    "cold-cache-restore": "cold_host_page_cache",
    "warm-cache-restore": "warm_host_page_cache",
    "prepared-worker": "warm_host_page_cache",
    "paused-pool": "warm_host_page_cache",
    "ready-pool": "warm_host_page_cache",
}
NETWORK_POLICY = "denied"
_FIXED_EXCLUDED_WORK = (
    "sandbox destruction, which every sample still executes and verifies after the timer stops",
    "the release build and its build manifest",
    "temporary state-root creation and removal",
    "host metadata collection and report generation",
)


@dataclass(frozen=True, slots=True)
class BurstPlan:
    """One experiment class measured at one concurrency with one workload."""

    experiment_class: str
    prepared_before_timer: tuple[str, ...]
    backend: str
    image: str
    command: tuple[str, ...]
    vcpus: int
    memory_mib: int
    storage_mib: int
    iterations: int
    concurrency: int
    timeout_ms: int
    max_output_bytes: int

    @classmethod
    def create(
        cls,
        *,
        experiment_class: str,
        prepared_before_timer: Sequence[str],
        backend: str,
        image: str,
        command: Sequence[str],
        vcpus: int,
        memory_mib: int,
        storage_mib: int,
        iterations: int,
        concurrency: int,
        timeout_ms: int,
        max_output_bytes: int,
    ) -> "BurstPlan":
        """Validate one declaration against the benchmark contract's class rules."""

        if experiment_class not in EXPERIMENT_CLASSES:
            raise ValueError(
                "experiment class must be one of: " + ", ".join(EXPERIMENT_CLASSES)
            )
        if backend not in BACKENDS:
            raise ValueError("backend must be one of: " + ", ".join(BACKENDS))
        prepared = _prepared(experiment_class, prepared_before_timer)
        if not image or any(character.isspace() for character in image):
            raise ValueError("image reference must be a nonempty value without whitespace")
        command = tuple(command)
        if not command or not command[0].startswith("/"):
            raise ValueError("workload command must start with an absolute guest executable")
        if any(not isinstance(value, str) or not value for value in command):
            raise ValueError("workload command arguments must be nonempty strings")
        _positive(vcpus, "vcpus")
        _positive(memory_mib, "memory mib")
        _positive(storage_mib, "storage mib")
        _positive(iterations, "iterations")
        _positive(concurrency, "concurrency")
        _positive(timeout_ms, "timeout ms")
        _positive(max_output_bytes, "max output bytes")
        if concurrency > iterations or iterations % concurrency != 0:
            raise ValueError("iterations must be a positive multiple of concurrency")
        return cls(
            experiment_class=experiment_class,
            prepared_before_timer=prepared,
            backend=backend,
            image=image,
            command=command,
            vcpus=vcpus,
            memory_mib=memory_mib,
            storage_mib=storage_mib,
            iterations=iterations,
            concurrency=concurrency,
            timeout_ms=timeout_ms,
            max_output_bytes=max_output_bytes,
        )

    @classmethod
    def from_dict(cls, value: Mapping[str, object]) -> "BurstPlan":
        """Rebuild and revalidate a declared plan from a retained record."""

        shape = value.get("shape")
        if not isinstance(shape, Mapping):
            raise ValueError("plan shape must be an object")
        return cls.create(
            experiment_class=str(value["experiment_class"]),
            prepared_before_timer=tuple(value["prepared_before_timer"]),
            backend=str(value["backend"]),
            image=str(value["image"]),
            command=tuple(value["command"]),
            vcpus=shape["vcpus"],
            memory_mib=shape["memory_mib"],
            storage_mib=shape["storage_mib"],
            iterations=value["iterations"],
            concurrency=value["concurrency"],
            timeout_ms=value["timeout_ms"],
            max_output_bytes=value["max_output_bytes"],
        )

    @property
    def cache_state(self) -> str:
        """Return the cache state the declared experiment class fixes."""

        return _CACHE_STATES[self.experiment_class]

    @property
    def bursts(self) -> int:
        """Return how many barrier-released groups the run opens."""

        return self.iterations // self.concurrency

    @property
    def excluded_work(self) -> tuple[str, ...]:
        """Return every operation the timer excludes, preparation included."""

        return _FIXED_EXCLUDED_WORK + tuple(
            f"preparation performed before the timer: {item}"
            for item in self.prepared_before_timer
        )

    def as_dict(self) -> dict[str, object]:
        return {
            "experiment_class": self.experiment_class,
            "preparation_class": self.experiment_class,
            "prepared_before_timer": list(self.prepared_before_timer),
            "cache_state": self.cache_state,
            "backend": self.backend,
            "image": self.image,
            "command": list(self.command),
            "network_policy": NETWORK_POLICY,
            "shape": {
                "vcpus": self.vcpus,
                "memory_mib": self.memory_mib,
                "storage_mib": self.storage_mib,
            },
            "iterations": self.iterations,
            "concurrency": self.concurrency,
            "bursts": self.bursts,
            "timeout_ms": self.timeout_ms,
            "max_output_bytes": self.max_output_bytes,
            "excluded_work": list(self.excluded_work),
        }


def _prepared(experiment_class: str, values: Sequence[str]) -> tuple[str, ...]:
    prepared = tuple(values)
    if any(
        not isinstance(value, str) or not value.strip() or "\n" in value
        for value in prepared
    ):
        raise ValueError("each declared preparation must be one nonempty line")
    if experiment_class == "cold-generation-build":
        if prepared:
            raise ValueError(
                "a cold-generation-build class must declare no preparation before the timer"
            )
        return ()
    if not prepared:
        raise ValueError(
            f"class {experiment_class} must declare what was prepared before the timer"
        )
    return prepared


def _positive(value: object, label: str) -> None:
    if type(value) is not int or value <= 0:
        raise ValueError(f"{label} must be a positive integer")
