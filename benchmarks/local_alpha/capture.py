"""Bounded external-process capture with a monotonic wall clock."""

from __future__ import annotations

import base64
import hashlib
import os
import signal
import subprocess
import threading
import time
from dataclasses import dataclass
from typing import BinaryIO, Mapping, Sequence


@dataclass(frozen=True, slots=True)
class StreamCapture:
    observed_bytes: int
    retained: bytes
    sha256: str
    truncated: bool

    def as_dict(self) -> dict[str, object]:
        return {
            "observed_bytes": self.observed_bytes,
            "retained_bytes": len(self.retained),
            "sha256": self.sha256,
            "truncated": self.truncated,
            "encoding": "base64",
            "data_base64": base64.b64encode(self.retained).decode("ascii"),
        }


@dataclass(frozen=True, slots=True)
class ProcessCapture:
    argv: tuple[str, ...]
    exit_code: int | None
    duration_ns: int
    harness_timed_out: bool
    stdout: StreamCapture
    stderr: StreamCapture

    def as_dict(self) -> dict[str, object]:
        return {
            "argv": list(self.argv),
            "clock": "time.perf_counter_ns",
            "boundary": "before_process_spawn_to_after_exit_and_pipe_drain",
            "duration_ns": self.duration_ns,
            "exit_code": self.exit_code,
            "harness_timed_out": self.harness_timed_out,
            "stdout": self.stdout.as_dict(),
            "stderr": self.stderr.as_dict(),
        }


class _Drain:
    def __init__(self, stream: BinaryIO, maximum_bytes: int) -> None:
        self._stream = stream
        self._maximum_bytes = maximum_bytes
        self._retained = bytearray()
        self._observed = 0
        self._digest = hashlib.sha256()
        self._failure: BaseException | None = None
        self._thread = threading.Thread(target=self._run, daemon=True)

    def start(self) -> None:
        self._thread.start()

    def finish(self) -> StreamCapture:
        self._thread.join(timeout=10)
        if self._thread.is_alive():
            raise RuntimeError("child output pipe did not close after process termination")
        if self._failure is not None:
            raise RuntimeError("failed to drain child output") from self._failure
        return StreamCapture(
            observed_bytes=self._observed,
            retained=bytes(self._retained),
            sha256=self._digest.hexdigest(),
            truncated=self._observed > self._maximum_bytes,
        )

    def _run(self) -> None:
        try:
            while True:
                chunk = self._stream.read(65_536)
                if not chunk:
                    return
                self._observed += len(chunk)
                self._digest.update(chunk)
                remaining = self._maximum_bytes - len(self._retained)
                if remaining > 0:
                    self._retained.extend(chunk[:remaining])
        except BaseException as error:
            self._failure = error


def _terminate_process_tree(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    if os.name == "posix":
        try:
            os.killpg(process.pid, signal.SIGKILL)
            return
        except ProcessLookupError:
            return
    process.kill()


def run_external_process(
    argv: Sequence[str],
    *,
    display_argv: Sequence[str],
    environment: Mapping[str, str],
    timeout_seconds: float,
    maximum_stream_bytes: int,
) -> ProcessCapture:
    """Execute one process and retain exact streams up to an explicit bound."""

    if not argv or not display_argv:
        raise ValueError("process argv must not be empty")
    if len(argv) != len(display_argv):
        raise ValueError("display argv must correspond exactly to process argv")
    if timeout_seconds <= 0 or maximum_stream_bytes <= 0:
        raise ValueError("capture bounds must be positive")

    start_ns = time.perf_counter_ns()
    process = subprocess.Popen(
        list(argv),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=dict(environment),
        start_new_session=os.name == "posix",
    )
    if process.stdout is None or process.stderr is None:
        _terminate_process_tree(process)
        raise RuntimeError("failed to create child output pipes")

    stdout_drain = _Drain(process.stdout, maximum_stream_bytes)
    stderr_drain = _Drain(process.stderr, maximum_stream_bytes)
    stdout_drain.start()
    stderr_drain.start()
    timed_out = False
    try:
        process.wait(timeout=timeout_seconds)
    except subprocess.TimeoutExpired:
        timed_out = True
        _terminate_process_tree(process)
        process.wait(timeout=5)

    stdout = stdout_drain.finish()
    stderr = stderr_drain.finish()
    process.stdout.close()
    process.stderr.close()
    end_ns = time.perf_counter_ns()
    return ProcessCapture(
        argv=tuple(display_argv),
        exit_code=process.returncode,
        duration_ns=end_ns - start_ns,
        harness_timed_out=timed_out,
        stdout=stdout,
        stderr=stderr,
    )
