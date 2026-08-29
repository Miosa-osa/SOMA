"""Shared identities and network translation for protocol plans."""

from __future__ import annotations

from collections.abc import Mapping

from benchmarks.local_alpha.matrix import Scenario

from .model import MACOS_DNS_SERVER


_NETWORK_FLAGS = {
    "denied": ("--egress", "denied", "--dns", "denied"),
    "unspecified": ("--egress", "unspecified", "--dns", "unspecified"),
    "allowed": (
        "--egress",
        "unrestricted",
        "--dns",
        "custom",
        "--dns-server",
        MACOS_DNS_SERVER,
    ),
}


def identity(value: str, label: str) -> str:
    valid = (
        isinstance(value, str)
        and len(value) == 32
        and all(character in "0123456789abcdef" for character in value)
        and set(value) != {"0"}
    )
    if not valid:
        raise ValueError(f"{label} must be a nonzero 32-character lowercase hex value")
    return value


def operations(scenario: Scenario) -> tuple[str, ...]:
    if scenario.mode == "one_shot":
        return ("run",)
    return ("launch", "exec", "destroy")


def operation_ids(
    scenario: Scenario, values: Mapping[str, str]
) -> dict[str, str]:
    return {
        name: identity(values.get(name, ""), f"{name} operation ID")
        for name in operations(scenario)
    }


def network(scenario: Scenario) -> tuple[tuple[str, ...], dict[str, object]]:
    try:
        flags = _NETWORK_FLAGS[scenario.network_policy]
    except KeyError as error:
        raise ValueError("unsupported network policy") from error
    body: dict[str, object] = {"egress": flags[1], "dns": flags[3]}
    if scenario.network_policy == "allowed":
        body["dns_servers"] = [MACOS_DNS_SERVER]
    return flags, body
