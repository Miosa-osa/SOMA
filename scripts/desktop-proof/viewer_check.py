#!/usr/bin/env python3
"""Checks the desktop viewer end to end through the SOMA tunnel, the way a browser reaches it.

The viewer is voxl on guest port 6080 (H.264 over one WebSocket; the session starts it). This opens
`GET /v1/sandboxes/{id}/tunnel/6080` through soma-api, asks for the browser client's page and its
script, each on a tunnel of its own, and requires the page to be the MIOSA Desktop viewer.
It is run more than the broker's idle timeout after the launch, which is the case #176 broke.

Usage: viewer_check.py --api HOST:PORT --tenant T --instance ID   (prints one JSON object)
"""

import argparse, json, sys, time  # noqa: E401

from viewer import Tunnel

PORT = 6080


def dechunk(body):
    """The body of a chunked HTTP/1.1 answer, without its size lines."""
    out = b""
    while body:
        size_line, _, body = body.partition(b"\r\n")
        size = int(size_line.split(b";")[0] or b"0", 16)
        if size == 0:
            break
        out, body = out + body[:size], body[size + 2:]
    return out


def http_get(args, path):
    started = time.monotonic()
    tunnel = Tunnel(args.api, args.tenant, args.instance, PORT)
    opened = time.monotonic()
    tunnel.sock.sendall(f"GET {path} HTTP/1.1\r\nhost: viewer\r\nconnection: close\r\n\r\n".encode())
    data = tunnel.rest
    while True:
        chunk = tunnel.sock.recv(65536)
        if not chunk:
            break
        data += chunk
    head, _, body = data.partition(b"\r\n\r\n")
    status = int(head.split(b" ", 2)[1]) if head.startswith(b"HTTP/") else None
    if b"transfer-encoding: chunked" in head.lower():
        body = dechunk(body)
    return status, body, round((opened - started) * 1000, 1), round((time.monotonic() - started) * 1000, 1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api", required=True)
    parser.add_argument("--tenant", required=True)
    parser.add_argument("--instance", required=True)
    args = parser.parse_args()
    result = {}
    try:
        status, body, tunnel_ms, page_ms = http_get(args, "/")
        result["page"] = {"status": status, "miosa_viewer": b"MIOSA Desktop" in body, "tunnel_open_ms": tunnel_ms,
                          "page_ms": page_ms}
        # The page's script, through a second tunnel: the client a browser loads next.
        scripts = [part.split(b'"')[0] for part in body.split(b'src="')[1:]]
        if scripts:
            status, body, _, script_ms = http_get(args, "/" + scripts[0].decode().lstrip("./"))
            result["script"] = {"status": status, "bytes": len(body), "ms": script_ms}
    except (ConnectionError, OSError, ValueError) as error:
        result["error"] = str(error)
    print(json.dumps(result))
    ok = (result.get("page", {}).get("status") == 200 and result["page"]["miosa_viewer"]
          and result.get("script", {"status": 200}).get("status") == 200)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
