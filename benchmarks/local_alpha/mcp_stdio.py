"""Bounded external-process MCP stdio client with correlated timing capture."""

from __future__ import annotations

import hashlib
import json
import queue
import subprocess
import threading
import time
from dataclasses import dataclass
from typing import Any, BinaryIO, Mapping, Sequence

from .capture import StreamCapture


MAXIMUM_FRAME_BYTES = 2 * 1024 * 1024
DEFAULT_MAXIMUM_STDERR_BYTES = 1024 * 1024


class _StderrDrain:
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
        self._thread.join(timeout=5)
        if self._thread.is_alive():
            raise RuntimeError("MCP stderr pipe did not close after process termination")
        if self._failure is not None:
            raise RuntimeError("failed to capture MCP stderr") from self._failure
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


@dataclass(frozen=True, slots=True)
class McpFrameCapture:
    request: dict[str, Any]
    response: dict[str, Any]
    duration_ns: int
    clock: str = "time.perf_counter_ns"

    def as_dict(self) -> dict[str, object]:
        return {
            "request": self.request,
            "response": self.response,
            "duration_ns": self.duration_ns,
            "clock": self.clock,
            "boundary": "before_jsonrpc_write_to_after_correlated_response_parse",
        }


class McpStdioSession:
    """Own one MCP server process and correlate concurrent JSON-RPC calls."""

    def __init__(
        self,
        argv: Sequence[str],
        *,
        display_argv: Sequence[str],
        environment: Mapping[str, str],
        response_timeout_seconds: float,
        maximum_stderr_bytes: int = DEFAULT_MAXIMUM_STDERR_BYTES,
    ) -> None:
        if not argv or not display_argv:
            raise ValueError("MCP argv and display argv must be nonempty")
        if response_timeout_seconds <= 0:
            raise ValueError("MCP response timeout must be positive")
        if maximum_stderr_bytes <= 0:
            raise ValueError("MCP stderr capture bound must be positive")
        self.argv = tuple(argv)
        self.display_argv = tuple(display_argv)
        self.environment = dict(environment)
        self.response_timeout_seconds = response_timeout_seconds
        self.maximum_stderr_bytes = maximum_stderr_bytes
        self._process: subprocess.Popen[bytes] | None = None
        self._next_identifier = 1
        self._identity_lock = threading.Lock()
        self._write_lock = threading.Lock()
        self._pending_lock = threading.Lock()
        self._pending: dict[int, queue.Queue[object]] = {}
        self._reader_failure: BaseException | None = None
        self._reader: threading.Thread | None = None
        self._stderr_drain: _StderrDrain | None = None
        self._stderr_capture: StreamCapture | None = None

    def __enter__(self) -> "McpStdioSession":
        self._process = subprocess.Popen(
            list(self.argv),
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=self.environment,
        )
        self._reader = threading.Thread(target=self._read_responses, daemon=True)
        if self._process.stderr is None:
            raise RuntimeError("MCP stderr is unavailable")
        self._stderr_drain = _StderrDrain(
            self._process.stderr, self.maximum_stderr_bytes
        )
        self._reader.start()
        self._stderr_drain.start()
        return self

    def __exit__(self, error_type: object, error: object, traceback: object) -> None:
        process = self._process
        if process is None:
            return
        if process.stdin is not None:
            process.stdin.close()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
        if self._reader is not None:
            self._reader.join(timeout=5)
        if self._stderr_drain is not None:
            self._stderr_capture = self._stderr_drain.finish()
        if process.stdout is not None:
            process.stdout.close()
        if process.stderr is not None:
            process.stderr.close()

    @property
    def stderr_capture(self) -> StreamCapture:
        if self._stderr_capture is None:
            raise RuntimeError("MCP stderr capture is available only after session close")
        return self._stderr_capture

    def initialize(self, protocol_version: str) -> McpFrameCapture:
        capture = self._call(
            "initialize",
            {
                "protocolVersion": protocol_version,
                "capabilities": {},
                "clientInfo": {"name": "soma-local-alpha-harness", "version": "1"},
            },
        )
        self._notify("notifications/initialized", {})
        return capture

    def call_tool(self, name: str, arguments: Mapping[str, Any]) -> McpFrameCapture:
        if not name:
            raise ValueError("MCP tool name must not be empty")
        return self._call("tools/call", {"name": name, "arguments": dict(arguments)})

    def _call(self, method: str, parameters: Mapping[str, Any]) -> McpFrameCapture:
        identifier = self._allocate_identifier()
        request = {
            "jsonrpc": "2.0",
            "id": identifier,
            "method": method,
            "params": dict(parameters),
        }
        mailbox: queue.Queue[object] = queue.Queue(maxsize=1)
        with self._pending_lock:
            self._pending[identifier] = mailbox
        started = time.perf_counter_ns()
        try:
            self._write_frame(request)
            response = mailbox.get(timeout=self.response_timeout_seconds)
        except queue.Empty as error:
            raise TimeoutError(f"MCP response timed out for {method}") from error
        finally:
            with self._pending_lock:
                self._pending.pop(identifier, None)
        duration = time.perf_counter_ns() - started
        if isinstance(response, BaseException):
            raise RuntimeError("MCP response reader failed") from response
        if not isinstance(response, dict):
            raise RuntimeError("MCP response was not an object")
        return McpFrameCapture(request=request, response=response, duration_ns=duration)

    def _notify(self, method: str, parameters: Mapping[str, Any]) -> None:
        self._write_frame(
            {"jsonrpc": "2.0", "method": method, "params": dict(parameters)}
        )

    def _allocate_identifier(self) -> int:
        with self._identity_lock:
            identifier = self._next_identifier
            self._next_identifier += 1
            return identifier

    def _write_frame(self, message: Mapping[str, Any]) -> None:
        process = self._require_process()
        if process.stdin is None:
            raise RuntimeError("MCP stdin is unavailable")
        encoded = (
            json.dumps(message, separators=(",", ":"), ensure_ascii=True) + "\n"
        ).encode("utf-8")
        if len(encoded) > MAXIMUM_FRAME_BYTES:
            raise ValueError("outbound MCP frame exceeds the harness bound")
        with self._write_lock:
            process.stdin.write(encoded)
            process.stdin.flush()

    def _read_responses(self) -> None:
        try:
            process = self._require_process()
            if process.stdout is None:
                raise RuntimeError("MCP stdout is unavailable")
            while True:
                line = process.stdout.readline(MAXIMUM_FRAME_BYTES + 1)
                if not line:
                    return
                if len(line) > MAXIMUM_FRAME_BYTES or not line.endswith(b"\n"):
                    raise RuntimeError("inbound MCP frame exceeds the harness bound")
                message = json.loads(line)
                identifier = message.get("id") if isinstance(message, dict) else None
                if not isinstance(identifier, int):
                    continue
                with self._pending_lock:
                    mailbox = self._pending.get(identifier)
                if mailbox is not None:
                    mailbox.put_nowait(message)
        except BaseException as error:
            self._reader_failure = error
            with self._pending_lock:
                mailboxes = list(self._pending.values())
            for mailbox in mailboxes:
                try:
                    mailbox.put_nowait(error)
                except queue.Full:
                    pass

    def _require_process(self) -> subprocess.Popen[bytes]:
        if self._process is None:
            raise RuntimeError("MCP session has not been started")
        return self._process
