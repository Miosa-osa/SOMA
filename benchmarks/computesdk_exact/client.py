"""One bounded JSON request to a SOMA public lifecycle endpoint."""

from __future__ import annotations

import http.client
import json
import socket
import ssl
from collections.abc import Mapping
from urllib.parse import urlsplit


class ApiClient:
    """A per-slot client whose requests include tenant identity."""

    def __init__(self, endpoint: str, tenant: str, timeout_seconds: float = 120.0):
        parsed = urlsplit(endpoint)
        if parsed.scheme not in {"http", "https"} or not parsed.hostname:
            raise ValueError("endpoint must be an absolute HTTP or HTTPS URL")
        if parsed.path not in {"", "/"} or parsed.query or parsed.fragment:
            raise ValueError("endpoint must not contain a path, query, or fragment")
        self._scheme = parsed.scheme
        self._host = parsed.hostname
        self._port = parsed.port
        self._tenant = tenant
        self._timeout = timeout_seconds
        self._session: http.client.HTTPConnection | None = None

    def request(
        self, method: str, path: str, body: Mapping[str, object] | None = None
    ) -> tuple[int, Mapping[str, object]]:
        """Send one request and require one bounded JSON object in response."""

        connection = self._session
        try:
            if connection is None:
                connection = self._connection()
                connection.connect()
                self._session = connection
            if connection.sock is None:
                raise OSError("HTTP connection opened without a socket")
            connection.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            encoded = (
                json.dumps(body, separators=(",", ":")) if body is not None else None
            )
            connection.request(
                method,
                path,
                body=encoded,
                headers={
                    "content-type": "application/json",
                    "x-soma-tenant": self._tenant,
                },
            )
            response = connection.getresponse()
            status = response.status
            payload = response.read(8 * 1024 * 1024 + 1)
        except Exception:
            connection.close()
            self._session = None
            raise
        if len(payload) > 8 * 1024 * 1024:
            raise ValueError("API response exceeds the benchmark capture bound")
        decoded = json.loads(payload)
        if not isinstance(decoded, Mapping):
            raise ValueError("API response is not one JSON object")
        return status, decoded

    def _connection(self) -> http.client.HTTPConnection:
        if self._scheme == "https":
            return http.client.HTTPSConnection(
                self._host,
                self._port,
                timeout=self._timeout,
                context=ssl.create_default_context(),
            )
        return http.client.HTTPConnection(self._host, self._port, timeout=self._timeout)
