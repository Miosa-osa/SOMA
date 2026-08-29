"""Deterministic fingerprints for Cargo inputs and benchmark harness code."""

from __future__ import annotations

import hashlib
from collections.abc import Iterable
from pathlib import Path


_ROOT_BUILD_FILES = ("Cargo.lock", "Cargo.toml", "rust-toolchain.toml")


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _fingerprint(root: Path, files: Iterable[Path], empty_message: str) -> str:
    ordered = tuple(sorted(files, key=lambda path: path.relative_to(root).as_posix()))
    if not ordered:
        raise ValueError(empty_message)
    digest = hashlib.sha256()
    for path in ordered:
        if path.is_symlink() or not path.is_file():
            raise ValueError("fingerprint inputs must be regular nonsymlink files")
        relative = path.relative_to(root).as_posix().encode("utf-8")
        content = path.read_bytes()
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        digest.update(len(content).to_bytes(8, "big"))
        digest.update(content)
    return digest.hexdigest()


def _cargo_files(root: Path) -> set[Path]:
    candidates: set[Path] = set()
    for name in _ROOT_BUILD_FILES:
        path = root / name
        if path.is_symlink():
            raise ValueError(f"Cargo input must not be a symlink: {name}")
        if path.is_file():
            candidates.add(path)
    for name in (".cargo", "crates"):
        directory = root / name
        if directory.is_symlink():
            raise ValueError(f"Cargo input directory must not be a symlink: {name}")
        if directory.is_dir():
            candidates.update(path for path in directory.rglob("*") if path.is_file())
    return candidates


def source_fingerprint(root: Path) -> str:
    """Hash every Cargo input while excluding docs and benchmark results."""

    root = root.resolve()
    return _fingerprint(root, _cargo_files(root), "no Cargo source files were found")


def benchmark_fingerprint(root: Path) -> str:
    """Hash Python benchmark code while excluding caches and generated results."""

    root = root.resolve()
    benchmark_root = root / "benchmarks"
    if benchmark_root.is_symlink():
        raise ValueError("benchmark harness directory must not be a symlink")
    files = (
        path
        for path in benchmark_root.rglob("*.py")
        if "__pycache__" not in path.parts and "results" not in path.parts
    )
    return _fingerprint(root, files, "no benchmark harness files were found")
