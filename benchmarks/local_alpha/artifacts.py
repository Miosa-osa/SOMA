"""Append-only raw evidence and atomic summary artifacts."""

from __future__ import annotations

import base64
import binascii
import json
import os
import threading
from collections.abc import Iterator
from pathlib import Path
from typing import IO, Any, Mapping


RAW_SCHEMA = "soma.local-alpha.raw.v1"
SUMMARY_SCHEMA = "soma.local-alpha.summary.v1"
MAXIMUM_RAW_RECORD_BYTES = 8 * 1024 * 1024
MAXIMUM_SUMMARY_BYTES = 2 * 1024 * 1024
_FORBIDDEN_KEY_PARTS = (
    "hardware_uuid",
    "host_uuid",
    "hostname",
    "ioplatformuuid",
    "machine_id",
    "platform_uuid",
    "serial_number",
)
_FORBIDDEN_VALUE_PARTS = (
    "IOPlatformUUID",
    "IOPlatformSerialNumber",
    "soma-local-alpha-state-",
)


def _validate_decoded_content(value: bytes, trail: tuple[str, ...]) -> None:
    lowered = value.lower()
    forbidden = tuple(part.encode("ascii") for part in _FORBIDDEN_KEY_PARTS)
    if any(part in lowered for part in forbidden):
        raise ValueError(f"base64 field contains hardware identity: {'.'.join(trail)}")
    home = os.fspath(Path.home()).encode()
    if home not in {b"", b"/"} and home in value:
        raise ValueError("base64 field contains a local home-directory path")
    if b"soma-local-alpha-state-" in lowered:
        raise ValueError("base64 field contains a private benchmark state path")
    try:
        nested = json.loads(value)
    except (UnicodeDecodeError, json.JSONDecodeError):
        return
    _validate_public_value(nested, trail)


def _validate_base64(value: object, trail: tuple[str, ...]) -> None:
    if not isinstance(value, str):
        raise ValueError(f"base64 field must be a string: {'.'.join(trail)}")
    try:
        decoded = base64.b64decode(value, validate=True)
    except (binascii.Error, ValueError) as error:
        raise ValueError(f"base64 field is invalid: {'.'.join(trail)}") from error
    _validate_decoded_content(decoded, trail)


def _validate_public_value(value: Any, trail: tuple[str, ...] = ()) -> None:
    if isinstance(value, Mapping):
        for key, nested in value.items():
            if not isinstance(key, str):
                raise ValueError("artifact object keys must be strings")
            lowered = key.lower()
            if any(part in lowered for part in _FORBIDDEN_KEY_PARTS):
                raise ValueError(f"hardware identity field is prohibited: {'.'.join(trail + (key,))}")
            encoded = lowered.endswith("_base64") or (
                lowered == "data" and value.get("encoding") == "base64"
            )
            if encoded:
                _validate_base64(nested, trail + (key,))
            _validate_public_value(nested, trail + (key,))
        return
    if isinstance(value, list) or isinstance(value, tuple):
        for index, nested in enumerate(value):
            _validate_public_value(nested, trail + (str(index),))
        return
    if isinstance(value, str):
        if any(part in value for part in _FORBIDDEN_VALUE_PARTS):
            raise ValueError("hardware identity value is prohibited")
        home = os.fspath(Path.home())
        if home and home != "/" and home in value:
            raise ValueError("artifact contains a local home-directory path")


def _encode(document: Mapping[str, Any]) -> bytes:
    _validate_public_value(document)
    return (
        json.dumps(document, sort_keys=True, separators=(",", ":"), ensure_ascii=True) + "\n"
    ).encode("utf-8")


def _sync_directory(path: Path) -> None:
    if os.name != "posix":
        return
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


class ArtifactWriter:
    """Own one never-overwritten evidence directory."""

    def __init__(self, destination: Path) -> None:
        self.destination = destination
        destination.mkdir(mode=0o700, parents=True, exist_ok=False)
        os.chmod(destination, 0o700)
        self._raw_path = destination / "raw.ndjson"
        descriptor = os.open(self._raw_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        self._raw: IO[bytes] = os.fdopen(descriptor, "wb", buffering=0)
        self._lock = threading.Lock()
        self._finished = False

    def __enter__(self) -> "ArtifactWriter":
        return self

    def __exit__(self, error_type: object, error: object, traceback: object) -> None:
        if not self._raw.closed:
            self._raw.close()

    def append(self, document: Mapping[str, Any]) -> None:
        if self._finished:
            raise RuntimeError("artifact writer is already finished")
        encoded = _encode(document)
        if len(encoded) > MAXIMUM_RAW_RECORD_BYTES:
            raise ValueError("raw artifact record exceeds its capture bound")
        with self._lock:
            self._raw.write(encoded)
            self._raw.flush()
            os.fsync(self._raw.fileno())

    def finish(self, summary: Mapping[str, Any]) -> None:
        if self._finished:
            raise RuntimeError("artifact writer is already finished")
        if summary.get("schema") != SUMMARY_SCHEMA:
            raise ValueError(f"summary schema must be {SUMMARY_SCHEMA}")
        encoded = _encode(summary)
        if len(encoded) > MAXIMUM_SUMMARY_BYTES:
            raise ValueError("summary artifact exceeds its capture bound")
        temporary = self.destination / ".summary.json.tmp"
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        try:
            with os.fdopen(descriptor, "wb") as stream:
                stream.write(encoded)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary, self.destination / "summary.json")
            _sync_directory(self.destination)
            self._finished = True
        finally:
            if temporary.exists():
                temporary.unlink()


def _raw_records(path: Path) -> Iterator[tuple[int, object]]:
    with path.open("rb") as stream:
        line_number = 0
        while True:
            encoded = stream.readline(MAXIMUM_RAW_RECORD_BYTES + 1)
            if not encoded:
                return
            line_number += 1
            if len(encoded) > MAXIMUM_RAW_RECORD_BYTES:
                raise ValueError(f"raw NDJSON line {line_number} exceeds its capture bound")
            try:
                yield line_number, json.loads(encoded)
            except (UnicodeDecodeError, json.JSONDecodeError) as error:
                raise ValueError(f"invalid raw NDJSON at line {line_number}") from error


def _bounded_json(path: Path, maximum_bytes: int, label: str) -> object:
    if path.stat().st_size > maximum_bytes:
        raise ValueError(f"{label} exceeds its capture bound")
    with path.open("rb") as stream:
        encoded = stream.read(maximum_bytes + 1)
    try:
        return json.loads(encoded)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{label} is not valid JSON") from error


def validate_artifact_directory(destination: Path) -> None:
    raw_path = destination / "raw.ndjson"
    summary_path = destination / "summary.json"
    if not raw_path.is_file() or not summary_path.is_file():
        raise ValueError("artifact directory is incomplete")
    if (destination / ".summary.json.tmp").exists():
        raise ValueError("artifact directory contains an interrupted summary")

    metadata_count = 0
    sample_ids: set[str] = set()
    run_id: str | None = None
    for line_number, record in _raw_records(raw_path):
        if not isinstance(record, dict):
            raise ValueError("raw NDJSON record must be an object")
        _validate_public_value(record)
        if record.get("schema") != RAW_SCHEMA:
            raise ValueError("raw record has an unknown schema")
        observed_run = record.get("run_id")
        if not isinstance(observed_run, str) or not observed_run:
            raise ValueError("raw record lacks a run identity")
        if run_id is None:
            run_id = observed_run
        elif observed_run != run_id:
            raise ValueError("raw records contain multiple run identities")
        if record.get("record_type") == "run_metadata":
            metadata_count += 1
        if record.get("record_type") == "sample":
            sample_id = record.get("sample_id")
            if not isinstance(sample_id, str) or not sample_id:
                raise ValueError("sample record lacks a sample identity")
            if sample_id in sample_ids:
                raise ValueError("sample identities must be unique")
            sample_ids.add(sample_id)
            duration = record.get("duration_ns")
            if not isinstance(duration, int) or duration < 0:
                raise ValueError("sample duration must be a nonnegative integer")
    if metadata_count != 1:
        raise ValueError("raw evidence must contain exactly one metadata record")

    summary = _bounded_json(summary_path, MAXIMUM_SUMMARY_BYTES, "summary")
    if not isinstance(summary, dict) or summary.get("schema") != SUMMARY_SCHEMA:
        raise ValueError("summary has an unknown schema")
    _validate_public_value(summary)
    if summary.get("run_id") != run_id:
        raise ValueError("summary run identity does not match raw evidence")
