"""Shared fixtures for burst harness behavior tests."""

from __future__ import annotations

import base64
import json
from collections.abc import Mapping, Sequence
from pathlib import Path

from benchmarks.local_alpha.burst.plan import BurstPlan
from benchmarks.local_alpha.burst.results import ResultsWriter


RUN_ID = "a1b2c3d4e5f60718293a4b5c6d7e8f90"
COMMAND = ("/bin/busybox", "true")
CLEANUP_COMPLETE = {
    "method": "forced",
    "machine": "complete",
    "memory": "complete",
    "storage": "complete",
    "guest_authority": "complete",
    "network": {
        "lease": "complete",
        "runtime_attachment": "complete",
        "address_leases": "complete",
        "egress_policy": "complete",
        "dns_policy": "complete",
        "proxy_policy": "complete",
        "ingress_bindings": "complete",
    },
}


def plan(**overrides: object) -> BurstPlan:
    arguments: dict[str, object] = {
        "experiment_class": "warm-cache-restore",
        "prepared_before_timer": ("the image was pulled before the timer",),
        "backend": "docker",
        "image": "busybox:stable-musl",
        "command": COMMAND,
        "vcpus": 1,
        "memory_mib": 1_024,
        "storage_mib": 10_240,
        "iterations": 2,
        "concurrency": 2,
        "timeout_ms": 30_000,
        "max_output_bytes": 1_048_576,
    }
    arguments.update(overrides)
    return BurstPlan.create(**arguments)  # type: ignore[arg-type]


def metadata_record(declared: BurstPlan) -> dict[str, object]:
    return {
        "record_type": "run_metadata",
        "run_id": RUN_ID,
        "started_at_utc": "2026-08-30T00:00:00+00:00",
        "plan": declared.as_dict(),
        "soma": {
            "git_revision": "b" * 40,
            "worktree_clean": True,
            "build_manifest": {
                "build_argv": ["cargo", "build"],
                "source_sha256": "c" * 64,
                "benchmark_sha256": "d" * 64,
                "binaries": {
                    "soma": {"filename": "soma", "sha256": "e" * 64, "size_bytes": 7}
                },
            },
        },
        "engine": {
            "schema": "soma.engine-settings.v1",
            "generation_store": {"state": "unset"},
            "head_directory": {"state": "unset"},
            "allow_uncertified_generation": False,
        },
        "host": {
            "kernel": {
                "sysname": "Linux",
                "release": "7.0.0-30-generic",
                "version": "#30",
                "machine": "x86_64",
            },
            "cpu": {"model": "Test CPU", "logical_cpus": 4, "microcode": "0x1"},
            "memory": {"total": "1 kB", "available_at_start": "1 kB"},
            "storage": {"state": "observed", "mount_point": "/", "filesystem": "ext4"},
            "kvm": {"device_present": True, "device_readable": True, "modules": ["kvm"]},
        },
        "backend_probe": {"exit_code": 0, "report": {"backend": "docker"}},
    }


def sample_record(
    declared: BurstPlan,
    repetition: int,
    *,
    successful: bool = True,
    tti_ns: int = 1_000,
    **overrides: object,
) -> dict[str, object]:
    record: dict[str, object] = {
        "record_type": "sample",
        "run_id": RUN_ID,
        "sample_id": f"{RUN_ID}-{repetition:06d}",
        "experiment_class": declared.experiment_class,
        "repetition": repetition,
        "burst_index": 0,
        "slot_index": repetition - 1,
        "boundary": "fixture-boundary",
        "clock": "time.perf_counter_ns",
        "tti_ns": tti_ns,
        "successful": successful,
        "command_succeeded": successful,
        "cleanup_complete": True,
        "instance_id": f"{repetition:032d}",
        "operation_ids": {},
        "processes": {},
        "stages": {
            "launch": [
                {"kind": "accepted", "elapsed_ns": 0},
                {"kind": "ready", "elapsed_ns": 400},
            ],
            "exec": [
                {"kind": "accepted", "elapsed_ns": 0},
                {"kind": "command_started", "elapsed_ns": 1},
                {"kind": "command_finished", "elapsed_ns": 300},
            ],
        },
        "observed": {
            "backend": "docker_container",
            "isolation": {"state": "observed", "value": "linux_container"},
            "workload": {"identity": {"manifest_digest": "sha256:" + "f" * 64}},
        },
        "command": {
            "status": "exited",
            "exit_code": 0,
            "stdout": {"encoding": "base64", "byte_length": 0, "data_base64": ""},
            "stderr": {"encoding": "base64", "byte_length": 0, "data_base64": ""},
        },
        "failures": []
        if successful
        else [{"reason": "command_unsuccessful", "operation": "exec", "detail": "17"}],
    }
    record.update(overrides)
    return record


def completion_record(
    declared: BurstPlan,
    attempted: int,
    *,
    failure_breakdown: Sequence[Mapping[str, object]] = (),
) -> dict[str, object]:
    return {
        "record_type": "run_completion",
        "run_id": RUN_ID,
        "finished_at_utc": "2026-08-30T00:00:01+00:00",
        "experiment_class": declared.experiment_class,
        "attempted": attempted,
        "wall_ns": 9_000,
        "failure_breakdown": list(failure_breakdown),
    }


def write_results(
    path: Path,
    declared: BurstPlan,
    samples: Sequence[Mapping[str, object]],
    *,
    metadata: Mapping[str, object] | None = None,
    completion: Mapping[str, object] | None = None,
) -> Path:
    with ResultsWriter(path) as writer:
        writer.append(metadata if metadata is not None else metadata_record(declared))
        for sample in samples:
            writer.append(sample)
        if completion is not None:
            writer.append(completion)
        else:
            writer.append(completion_record(declared, len(samples)))
    return path


def complete_results(path: Path, declared: BurstPlan | None = None) -> Path:
    declared = declared or plan()
    samples = [
        sample_record(declared, index, tti_ns=1_000 * index)
        for index in range(1, declared.iterations + 1)
    ]
    return write_results(path, declared, samples)


def envelope(
    command: str,
    instance_id: str,
    *,
    result: Mapping[str, object],
    milestones: Sequence[Mapping[str, object]],
    status: str = "ok",
    receipt: Mapping[str, object] | None = None,
) -> bytes:
    body = {
        "schema": "soma.cli.v1",
        "command": command,
        "status": status,
        "result": {"instance_id": instance_id, **result},
        "error": None,
        "receipt": {
            "instance_id": instance_id,
            "milestones": list(milestones),
            **(receipt or {}),
        },
    }
    return json.dumps(body).encode("utf-8")


def encoded(value: bytes) -> dict[str, object]:
    return {
        "encoding": "base64",
        "byte_length": len(value),
        "data": base64.b64encode(value).decode("ascii"),
    }
