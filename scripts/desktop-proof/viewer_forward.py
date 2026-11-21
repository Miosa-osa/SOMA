#!/usr/bin/env python3
"""Serves a SOMA desktop's viewer port on a local TCP port, one SOMA tunnel per connection.

A browser pointed at http://<listen>/ loads KasmVNC's own web client from the guest and opens its
WebSocket on the same address, so every byte it exchanges with the desktop crosses
`GET /v1/sandboxes/{id}/tunnel/6080`. This is the human viewer of the proof; it adds no protocol
of its own and holds no state between connections.

Usage: viewer_forward.py --instance ID [--listen 0.0.0.0:16080] [--api 127.0.0.1:18930]
"""

import argparse, socket, sys, threading

from viewer import Tunnel


def pump(source, sink):
    try:
        while True:
            data = source.recv(65536)
            if not data:
                break
            sink.sendall(data)
    except OSError:
        pass
    finally:
        for end in (sink, source):
            try:
                end.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass


def serve(client, args):
    try:
        tunnel = Tunnel(args.api, args.tenant, args.instance, args.port)
    except (OSError, ConnectionError) as error:
        print(f"tunnel refused: {error}", file=sys.stderr, flush=True)
        client.close()
        return
    tunnel.sock.settimeout(None)
    if tunnel.rest:
        client.sendall(tunnel.rest)
    threading.Thread(target=pump, args=(client, tunnel.sock), daemon=True).start()
    pump(tunnel.sock, client)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--instance", required=True)
    parser.add_argument("--listen", default="0.0.0.0:16080")
    parser.add_argument("--api", default="127.0.0.1:18930")
    parser.add_argument("--tenant", default="m1")
    parser.add_argument("--port", type=int, default=6080)
    args = parser.parse_args()
    host, _, port = args.listen.rpartition(":")
    listener = socket.create_server((host, int(port)), reuse_port=True)
    print(f"viewer on http://{args.listen}/ -> sandbox {args.instance} port {args.port}", flush=True)
    while True:
        client, _ = listener.accept()
        client.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        threading.Thread(target=serve, args=(client, args), daemon=True).start()


if __name__ == "__main__":
    sys.exit(main())
