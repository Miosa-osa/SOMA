"""One exact create-through-first-command sample with excluded cleanup."""

from __future__ import annotations

import base64
import time
from collections.abc import Callable, Mapping
from threading import Barrier

CREATE = {
    "image": "node:22",
    "shape": {
        "vcpu_count": 1,
        "memory_mib": 1024,
        "storage_mib": 4096,
        "capabilities": {
            "network": {
                "profile": {"mode": "disabled"},
                "guest_addresses": {
                    "ipv4": {"mode": "disabled"},
                    "ipv6": {"mode": "disabled"},
                },
                "proxy": {"mode": "disabled"},
                "egress": "denied",
                "dns": {"mode": "denied"},
                "published_ports": [],
            }
        },
    },
}
COMMAND = {"executable": "/usr/local/bin/node", "arguments": ["-v"]}


def execute_slot(
    client: object,
    barrier: Barrier,
    *,
    clock: Callable[[], int] = time.perf_counter_ns,
    release_at_epoch_ns: int | None = None,
) -> dict[str, object]:
    """Create, run Node, stop timing, then destroy and retain every outcome."""

    instance: str | None = None
    failure: str | None = None
    cleanup_complete = False
    started_ns = create_finished_ns = tti_finished_ns = None
    cleanup_finished_ns = None
    preparation = None
    launch_milestones = None
    command_milestones = None
    try:
        barrier.wait(timeout=300)
        _await_release(release_at_epoch_ns)
        started_ns = clock()
        status, launch = client.request("POST", "/v1/sandboxes", CREATE)
        create_finished_ns = clock()
        instance, preparation, launch_milestones = _launch(status, launch)
        status, command = client.request(
            "POST", f"/v1/sandboxes/{instance}/commands", COMMAND
        )
        command_milestones = _command(status, command, instance)
        tti_finished_ns = clock()
    except Exception as error:
        failure = f"{type(error).__name__}: {error}"
        if started_ns is not None and tti_finished_ns is None:
            tti_finished_ns = clock()
    finally:
        if instance is not None:
            try:
                status, cleanup = client.request("DELETE", f"/v1/sandboxes/{instance}")
                cleanup_complete = _cleanup(status, cleanup, instance)
            except Exception as error:
                if failure is None:
                    failure = f"cleanup {type(error).__name__}: {error}"
            cleanup_finished_ns = clock()
    command_succeeded = failure is None or failure.startswith("cleanup ")
    return {
        "instance_id": instance,
        "started_ns": started_ns,
        "tti_finished_ns": tti_finished_ns,
        "cleanup_finished_ns": cleanup_finished_ns,
        "create_ns": _delta(started_ns, create_finished_ns),
        "tti_ns": _delta(started_ns, tti_finished_ns),
        "preparation": preparation,
        "launch_milestones": launch_milestones,
        "command_milestones": command_milestones,
        "command_succeeded": command_succeeded,
        "cleanup_complete": cleanup_complete,
        "successful": command_succeeded and cleanup_complete,
        "failure": failure,
    }


def _launch(
    status: int, envelope: Mapping[str, object]
) -> tuple[str, object, object]:
    result = envelope.get("result")
    if status != 201 or not _okay(envelope) or not isinstance(result, Mapping):
        error = envelope.get("error")
        code = error.get("code") if isinstance(error, Mapping) else None
        raise ValueError(f"create refused with HTTP {status}, code {code!r}")
    instance = result.get("instance_id")
    if result.get("state") != "ready" or not isinstance(instance, str):
        raise ValueError("create did not return one ready Instance")
    receipt = envelope.get("receipt")
    preparation = receipt.get("preparation") if isinstance(receipt, Mapping) else None
    return instance, preparation, _milestones(receipt)


def _command(status: int, envelope: Mapping[str, object], instance: str) -> object:
    result = envelope.get("result")
    if status != 200 or not _okay(envelope) or not isinstance(result, Mapping):
        raise ValueError(f"command refused with HTTP {status}")
    if result.get("instance_id") != instance:
        raise ValueError("command answered for another Instance")
    execution = result.get("execution")
    if not isinstance(execution, Mapping) or execution.get("exited") != {"code": 0}:
        raise ValueError("node -v did not exit zero")
    stdout = result.get("stdout")
    if not isinstance(stdout, Mapping) or stdout.get("encoding") != "base64":
        raise ValueError("node -v stdout is not explicit base64")
    output = base64.b64decode(str(stdout.get("data", "")), validate=True)
    if not output.startswith(b"v"):
        raise ValueError("node -v returned no version")
    return _milestones(envelope.get("receipt"))


def _milestones(receipt: object) -> object:
    if not isinstance(receipt, Mapping):
        return None
    milestones = receipt.get("milestones")
    return milestones if isinstance(milestones, list) else None


def _cleanup(status: int, envelope: Mapping[str, object], instance: str) -> bool:
    result = envelope.get("result")
    receipt = envelope.get("receipt")
    return (
        status == 200
        and _okay(envelope)
        and isinstance(result, Mapping)
        and result.get("instance_id") == instance
        and result.get("state") == "destroyed"
        and isinstance(receipt, Mapping)
        and _cleanup_complete(receipt)
    )


def _okay(envelope: Mapping[str, object]) -> bool:
    return envelope.get("schema") == "soma.api.v1" and envelope.get("status") == "ok"


def _cleanup_complete(receipt: Mapping[str, object]) -> bool:
    cleanup = receipt.get("cleanup")
    if not isinstance(cleanup, Mapping):
        return False
    if cleanup.get("method") not in {"graceful", "forced", "graceful_then_forced"}:
        return False
    if any(
        cleanup.get(resource) != "complete"
        for resource in ("machine", "memory", "storage", "guest_authority")
    ):
        return False
    network = cleanup.get("network")
    keys = {
        "lease", "runtime_attachment", "address_leases", "egress_policy",
        "dns_policy", "proxy_policy", "ingress_bindings",
    }
    return (
        isinstance(network, Mapping)
        and set(network) == keys
        and all(value in {"complete", "not_owned"} for value in network.values())
    )


def _delta(start: int | None, finish: int | None) -> int | None:
    return None if start is None or finish is None else finish - start


def _await_release(release_at_epoch_ns: int | None) -> None:
    while release_at_epoch_ns is not None:
        remaining = release_at_epoch_ns - time.time_ns()
        if remaining <= 0:
            return
        time.sleep(min(remaining / 1_000_000_000, 0.01))
