"""Secret-minimized child process environments."""

from __future__ import annotations

from collections.abc import Mapping


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
