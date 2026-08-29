#!/usr/bin/env python3
"""Build the static x86_64 PID 1 used by the ignored PVH kernel-boot test.

The binary is compiled inside the pinned ``soma-kernel-builder:local`` image when Docker and
that image are available, so it uses the same gcc as the pinned kernel. Otherwise the native
``cc`` is used and reported. Set ``SOMA_X86_64_INIT_BUILDER`` to ``docker`` or ``native`` to
force one path. The output is never committed; the test packs it into a ``newc`` archive.
"""

from __future__ import annotations

import os
import platform
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

BUILDER_IMAGE = "soma-kernel-builder:local"
CFLAGS = [
    "-static",
    "-nostdlib",
    "-ffreestanding",
    "-fno-builtin",
    "-fno-stack-protector",
    "-fno-tree-loop-distribute-patterns",
    "-fno-pie",
    "-no-pie",
    "-fcf-protection=none",
    "-O2",
    "-Wall",
    "-Wextra",
    "-Wl,--build-id=none",
    "-Wl,-z,noexecstack",
    "-Wl,-s",
    "-Wl,-e,_start",
]


def _docker_available() -> bool:
    if shutil.which("docker") is None:
        return False
    probe = subprocess.run(
        ["docker", "image", "inspect", BUILDER_IMAGE],
        capture_output=True,
        check=False,
    )
    return probe.returncode == 0


def _compile(source: Path, output: Path, builder: str) -> None:
    if builder == "docker":
        workdir = output.parent
        shutil.copy(source, workdir / source.name)
        command = [
            "docker",
            "run",
            "--rm",
            "--user",
            f"{os.getuid()}:{os.getgid()}",
            "-v",
            f"{workdir}:/work",
            "-w",
            "/work",
            BUILDER_IMAGE,
            "gcc",
            *CFLAGS,
            source.name,
            "-o",
            output.name,
        ]
    else:
        command = ["cc", *CFLAGS, os.fspath(source), "-o", os.fspath(output)]
    subprocess.run(command, check=True)


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: build_x86_64_init.py /absolute/output/path")
    output = Path(sys.argv[1])
    if platform.machine() != "x86_64":
        raise SystemExit("the fixture must be built on an x86_64 host")
    if not output.is_absolute() or not output.parent.is_dir():
        raise SystemExit("output must be an absolute path below an existing directory")
    if os.path.lexists(output):
        raise SystemExit("output must not already exist")

    builder = os.environ.get("SOMA_X86_64_INIT_BUILDER", "")
    if builder not in {"docker", "native"}:
        builder = "docker" if _docker_available() else "native"

    source = Path(__file__).with_name("x86_64_init.c")
    with tempfile.TemporaryDirectory(prefix="soma-kvm-x86-init-", dir=output.parent) as temporary:
        executable = Path(temporary) / "init"
        _compile(source, executable, builder)
        init = executable.read_bytes()
    if init[:4] != b"\x7fELF" or struct.unpack_from("<H", init, 18)[0] != 62:
        raise SystemExit("compiler did not produce a Linux x86_64 ELF executable")
    if struct.unpack_from("<H", init, 16)[0] != 2:
        raise SystemExit("compiler did not produce an ET_EXEC executable")

    temporary_output = output.with_name(f".{output.name}.{os.getpid()}.tmp")
    try:
        with temporary_output.open("xb") as stream:
            stream.write(init)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary_output, output)
    finally:
        temporary_output.unlink(missing_ok=True)
    print(f"builder={builder} bytes={len(init)} output={output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
