"""Host, kernel, storage, and backend identity bound into every burst result."""

from __future__ import annotations

import json
import os
from collections.abc import Mapping
from datetime import UTC, datetime
from pathlib import Path

from benchmarks.local_alpha.capture import run_external_process
from benchmarks.local_alpha.provenance import BuildManifest

from .plan import BurstPlan


PROBE_TIMEOUT_SECONDS = 60.0
PROBE_OUTPUT_BYTES = 64 * 1024


def collect(
    plan: BurstPlan,
    *,
    run_id: str,
    manifest: BuildManifest,
    soma_binary: Path,
    state_root: Path,
    environment: Mapping[str, str],
) -> dict[str, object]:
    """Collect every metadata field the benchmark contract requires."""

    return {
        "run_id": run_id,
        "started_at_utc": datetime.now(UTC).isoformat(),
        "plan": plan.as_dict(),
        "soma": {
            "git_revision": manifest.git_revision,
            "worktree_clean": manifest.worktree_clean,
            "build_manifest": manifest.as_dict(),
        },
        "host": {
            "kernel": _kernel(),
            "cpu": _cpu(),
            "memory": _memory(),
            "storage": _storage(state_root),
            "kvm": _kvm(),
        },
        "backend_probe": _probe(plan, soma_binary, environment),
    }


def _kernel() -> dict[str, object]:
    uname = os.uname()
    return {
        "sysname": uname.sysname,
        "release": uname.release,
        "version": uname.version,
        "machine": uname.machine,
    }


def _cpu() -> dict[str, object]:
    fields = _first_fields(
        Path("/proc/cpuinfo"), ("model name", "microcode", "cpu family", "vendor_id")
    )
    return {
        "model": fields.get("model name"),
        "vendor": fields.get("vendor_id"),
        "family": fields.get("cpu family"),
        "microcode": fields.get("microcode"),
        "logical_cpus": os.cpu_count(),
    }


def _memory() -> dict[str, object]:
    fields = _first_fields(Path("/proc/meminfo"), ("MemTotal", "MemAvailable"), ":")
    return {
        "total": fields.get("MemTotal"),
        "available_at_start": fields.get("MemAvailable"),
    }


def _storage(state_root: Path) -> dict[str, object]:
    mount = _mount_entry(state_root)
    if mount is None:
        return {"state": "unavailable"}
    device_number = mount["device_number"]
    return {
        "state": "observed",
        "mount_point": mount["mount_point"],
        "filesystem": mount["filesystem"],
        "source": mount["source"],
        "mount_options": mount["mount_options"],
        "super_options": mount["super_options"],
        "device_number": device_number,
        "device_model": _device_model(device_number),
    }


def _mount_entry(path: Path) -> dict[str, str] | None:
    try:
        lines = Path("/proc/self/mountinfo").read_text(encoding="utf-8").splitlines()
    except OSError:
        return None
    resolved = path.resolve()
    best: dict[str, str] | None = None
    for line in lines:
        head, _, tail = line.partition(" - ")
        head_fields = head.split(" ")
        tail_fields = tail.split(" ")
        if len(head_fields) < 6 or len(tail_fields) < 3:
            continue
        mount_point = Path(head_fields[4])
        if resolved != mount_point and mount_point not in resolved.parents:
            continue
        if best is not None and len(str(mount_point)) <= len(best["mount_point"]):
            continue
        best = {
            "device_number": head_fields[2],
            "mount_point": head_fields[4],
            "mount_options": head_fields[5],
            "filesystem": tail_fields[0],
            "source": tail_fields[1],
            "super_options": tail_fields[2],
        }
    return best


def _device_model(device_number: str) -> str | None:
    base = Path("/sys/dev/block") / device_number
    for candidate in (base / "device" / "model", base / ".." / "device" / "model"):
        try:
            return candidate.read_text(encoding="utf-8").strip()
        except OSError:
            continue
    return None


def _kvm() -> dict[str, object]:
    device = Path("/dev/kvm")
    modules = []
    try:
        for line in Path("/proc/modules").read_text(encoding="utf-8").splitlines():
            name = line.split(" ", 1)[0]
            if name.startswith("kvm"):
                modules.append(name)
    except OSError:
        modules = []
    return {
        "device_present": device.exists(),
        "device_readable": os.access(device, os.R_OK | os.W_OK),
        "modules": sorted(modules),
    }


def _probe(
    plan: BurstPlan, soma_binary: Path, environment: Mapping[str, str]
) -> dict[str, object]:
    argv = (
        os.fspath(soma_binary),
        "--format",
        "json",
        "--backend",
        plan.backend,
        "doctor",
    )
    capture = run_external_process(
        argv,
        display_argv=("$SOMA_BIN", *argv[1:]),
        environment=environment,
        timeout_seconds=PROBE_TIMEOUT_SECONDS,
        maximum_stream_bytes=PROBE_OUTPUT_BYTES,
    )
    report: object = None
    try:
        report = json.loads(capture.stdout.retained).get("result")
    except (UnicodeDecodeError, json.JSONDecodeError, AttributeError):
        report = None
    return {"exit_code": capture.exit_code, "report": report}


def _first_fields(
    path: Path, names: tuple[str, ...], separator: str = ":"
) -> dict[str, str]:
    values: dict[str, str] = {}
    try:
        text = path.read_text(encoding="utf-8")
    except OSError:
        return values
    for line in text.splitlines():
        name, found, value = line.partition(separator)
        if not found:
            continue
        name = name.strip()
        if name in names and name not in values:
            values[name] = value.strip()
    return values
