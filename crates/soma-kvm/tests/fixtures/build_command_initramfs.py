#!/usr/bin/env python3
"""Build the deterministic ARM64 command-agent initramfs for ignored KVM tests."""

from __future__ import annotations

import os
import platform
import struct
import subprocess
import sys
import tempfile
from pathlib import Path


def _entry(name: str, data: bytes, mode: int, inode: int) -> bytes:
    encoded_name = name.encode("ascii") + b"\0"
    fields = (inode, mode, 0, 0, 1, 0, len(data), 0, 0, 0, 0, len(encoded_name), 0)
    header = b"070701" + b"".join(f"{field:08x}".encode("ascii") for field in fields)
    body = header + encoded_name
    body += b"\0" * (-len(body) % 4)
    body += data
    return body + b"\0" * (-len(body) % 4)


def _compile(sources: list[Path], output: Path) -> bytes:
    subprocess.run(
        [
            "cc",
            "-std=c11",
            "-Os",
            "-static",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-fno-ident",
            "-Wl,--build-id=none",
            "-Wl,-z,noexecstack",
            "-Wl,-s",
            *(os.fspath(source) for source in sources),
            "-o",
            os.fspath(output),
        ],
        check=True,
        env={**os.environ, "LC_ALL": "C", "SOURCE_DATE_EPOCH": "0"},
    )
    binary = output.read_bytes()
    if binary[:4] != b"\x7fELF" or struct.unpack_from("<H", binary, 18)[0] != 183:
        raise SystemExit("compiler did not produce a Linux ARM64 ELF executable")
    return binary


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: build_command_initramfs.py /absolute/output/path")
    output = Path(sys.argv[1])
    if platform.machine() not in {"aarch64", "arm64"}:
        raise SystemExit("the fixture must be built natively on ARM64")
    if not output.is_absolute() or not output.parent.is_dir():
        raise SystemExit("output must be an absolute path below an existing directory")
    if os.path.lexists(output):
        raise SystemExit("output must not already exist")

    with tempfile.TemporaryDirectory(prefix="soma-kvm-command-") as temporary:
        directory = Path(temporary)
        init = _compile(
            [
                Path(__file__).with_name("arm64_agent.c"),
                Path(__file__).with_name("arm64_process.c"),
            ],
            directory / "init",
        )
        probe = _compile([Path(__file__).with_name("arm64_probe.c")], directory / "probe")
        process_test = directory / "process-test"
        _compile(
            [
                Path(__file__).with_name("arm64_process_test.c"),
                Path(__file__).with_name("arm64_process.c"),
            ],
            process_test,
        )
        subprocess.run([process_test], check=True)

    archive = _entry("dev", b"", 0o040755, 1)
    archive += _entry("init", init, 0o100755, 2)
    archive += _entry("probe", probe, 0o100755, 3)
    archive += _entry("TRAILER!!!", b"", 0, 4)
    temporary_output = output.with_name(f".{output.name}.{os.getpid()}.tmp")
    try:
        with temporary_output.open("xb") as stream:
            stream.write(archive)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary_output, output)
    finally:
        temporary_output.unlink(missing_ok=True)
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
