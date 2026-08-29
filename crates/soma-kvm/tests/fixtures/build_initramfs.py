#!/usr/bin/env python3
"""Build the deterministic ARM64 PID1 initramfs used by the ignored KVM test."""

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


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: build_initramfs.py /absolute/output/path")
    output = Path(sys.argv[1])
    if platform.machine() not in {"aarch64", "arm64"}:
        raise SystemExit("the fixture must be built natively on ARM64")
    if not output.is_absolute() or not output.parent.is_dir():
        raise SystemExit("output must be an absolute path below an existing directory")
    if os.path.lexists(output):
        raise SystemExit("output must not already exist")

    source = Path(__file__).with_name("arm64_init.S")
    with tempfile.TemporaryDirectory(prefix="soma-kvm-init-") as temporary:
        executable = Path(temporary) / "init"
        subprocess.run(
            [
                "cc",
                "-nostdlib",
                "-static",
                "-Wl,--build-id=none",
                "-Wl,-z,noexecstack",
                "-Wl,-s",
                "-Wl,-e,_start",
                os.fspath(source),
                "-o",
                os.fspath(executable),
            ],
            check=True,
        )
        init = executable.read_bytes()
        if init[:4] != b"\x7fELF" or struct.unpack_from("<H", init, 18)[0] != 183:
            raise SystemExit("compiler did not produce a Linux ARM64 ELF executable")

    archive = _entry("init", init, 0o100755, 1)
    archive += _entry("TRAILER!!!", b"", 0, 2)
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
