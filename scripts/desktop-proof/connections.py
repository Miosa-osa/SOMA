#!/usr/bin/env python3
"""Lists the remote endpoints in a guest's /proc/net/tcp and /proc/net/udp, loopback excluded.

Usage: connections.py <file holding /proc/net/tcp followed by /proc/net/udp>
Prints one line per socket with a non-loopback peer, then a count. Listening sockets and sockets
with no peer have the address 0.0.0.0:0 as their peer and are not listed.
"""

import socket, struct, sys

STATES = {"01": "established", "02": "syn-sent", "06": "time-wait", "07": "close", "0A": "listen"}


def endpoint(text):
    address, port = text.split(":")
    return socket.inet_ntoa(struct.pack("<I", int(address, 16))), int(port, 16)


def main():
    remote = []
    for line in open(sys.argv[1], encoding="utf-8"):
        fields = line.split()
        if len(fields) < 4 or fields[0] == "sl":
            continue
        local, peer = endpoint(fields[1]), endpoint(fields[2])
        if peer[0] in ("0.0.0.0",) or peer[0].startswith("127."):
            continue
        remote.append(f"{local[0]}:{local[1]} -> {peer[0]}:{peer[1]} {STATES.get(fields[3], fields[3])}")
    for entry in remote:
        print(entry)
    print(f"{len(remote)} socket(s) with a non-loopback peer")


if __name__ == "__main__":
    main()
