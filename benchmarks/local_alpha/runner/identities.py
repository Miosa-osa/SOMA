"""Collision-checked benchmark identities."""

from __future__ import annotations

import secrets
from collections.abc import Callable


class IdentityGenerator:
    def __init__(self, generate: Callable[[], str] | None = None) -> None:
        self._generate = generate or (lambda: secrets.token_hex(16))
        self._used: set[str] = set()

    def new(self) -> str:
        for _ in range(1_024):
            candidate = self._generate()
            valid = (
                isinstance(candidate, str)
                and len(candidate) == 32
                and all(character in "0123456789abcdef" for character in candidate)
                and set(candidate) != {"0"}
            )
            if not valid:
                raise ValueError("generated identity must be nonzero 32-character lowercase hex")
            if candidate not in self._used:
                self._used.add(candidate)
                return candidate
        raise RuntimeError("identity source repeatedly returned used values")
