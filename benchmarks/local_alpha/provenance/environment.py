"""Secret-minimized child process environments."""

from __future__ import annotations

from collections.abc import Mapping
from hashlib import sha256


_ALLOWLIST = frozenset(
    {
        "HOME",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "PATH",
        "SSL_CERT_DIR",
        "SSL_CERT_FILE",
        "TMPDIR",
        "TZ",
    }
)
_SECRET_MARKERS = ("API_KEY", "PASSWORD", "SECRET", "TOKEN", "CREDENTIAL")

# Runtime settings the development KVM Backend needs in order to serve a request.
# Build-tool paths remain outside measured children because Generation construction is preparation.
ENGINE_SETTINGS = (
    "SOMA_GENERATION_STORE",
    "SOMA_HEAD_DIR",
    "SOMA_ALLOW_UNCERTIFIED_GENERATION",
)


def engine_settings(source: Mapping[str, str]) -> dict[str, str]:
    """Return the engine settings present in `source`, for explicit forwarding."""

    return {name: source[name] for name in ENGINE_SETTINGS if source.get(name)}


def engine_setting_provenance(settings: Mapping[str, str]) -> dict[str, object]:
    """Return non-secret identities for the effective development-engine settings."""

    return {
        "schema": "soma.engine-settings.v1",
        "generation_store": _locator_identity(settings.get("SOMA_GENERATION_STORE")),
        "head_directory": _locator_identity(settings.get("SOMA_HEAD_DIR")),
        "allow_uncertified_generation": settings.get(
            "SOMA_ALLOW_UNCERTIFIED_GENERATION"
        )
        == "1",
    }


def _locator_identity(value: str | None) -> dict[str, str]:
    if not value:
        return {"state": "unset"}
    return {
        "state": "configured",
        "locator_sha256": sha256(value.encode("utf-8")).hexdigest(),
    }


def _secret_bearing(name: str) -> bool:
    uppercase = name.upper()
    return any(marker in uppercase for marker in _SECRET_MARKERS)


def build_child_environment(
    source: Mapping[str, str],
    explicit: Mapping[str, str] | None = None,
) -> dict[str, str]:
    """Return runtime essentials and reviewed non-secret explicit values."""

    environment = {
        name: value for name, value in source.items() if name in _ALLOWLIST and value
    }
    for name, value in (explicit or {}).items():
        if not name or "=" in name or "\x00" in name or "\x00" in value:
            raise ValueError("invalid explicit child environment entry")
        if _secret_bearing(name):
            raise ValueError("secret-bearing environment entries are prohibited")
        environment[name] = value
    return environment
