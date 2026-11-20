#!/usr/bin/env python3
"""Times a SOMA desktop's first frame from inside the guest, as a viewer of port 6080 sees it.

Started in the same command as the session launcher, it retries until KasmVNC answers on
127.0.0.1:6080, then speaks WebSocket and RFB 3.8 (security type None, 32-bit true colour, Raw
encoding only, so every pixel is seen) and prints one JSON line of guest wall-clock stamps in
nanoseconds:

  probe_start_ns     when it started trying
  rfb_ready_ns       the RFB handshake finished
  first_update_ns    the first framebuffer update arrived
  first_painted_ns   the first frame that is not one flat colour (the desktop is drawn)

Only the Python standard library is used, so it runs in any guest with python3.

Usage: first_frame.py [--port 6080] [--deadline 60]
"""

import argparse, base64, json, os, socket, struct, sys, time


def now():
    return time.time_ns()


class Viewer:
    def __init__(self, port):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=10)
        key = base64.b64encode(os.urandom(16)).decode()
        # KasmVNC closes an upgrade that names no Origin, as a browser always does.
        self.sock.sendall((
            "GET /websockify HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n"
            f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n"
            "Origin: http://localhost\r\nSec-WebSocket-Protocol: binary\r\n\r\n").encode())
        self.buf, self.payload = b"", b""
        while b"\r\n\r\n" not in self.buf:
            self._recv()
        head, self.buf = self.buf.split(b"\r\n\r\n", 1)
        if b" 101 " not in head.split(b"\r\n")[0]:
            raise ConnectionError(f"WebSocket refused: {head[:120]!r}")

    def _recv(self):
        chunk = self.sock.recv(1 << 20)
        if not chunk:
            raise ConnectionError("viewer port closed the stream")
        self.buf += chunk

    def _raw(self, n):
        while len(self.buf) < n:
            self._recv()
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def read(self, n):
        while len(self.payload) < n:
            b0, b1 = self._raw(2)
            length = b1 & 0x7F
            if length == 126:
                length = struct.unpack(">H", self._raw(2))[0]
            elif length == 127:
                length = struct.unpack(">Q", self._raw(8))[0]
            data = self._raw(length)
            if b0 & 0x0F == 8:
                raise ConnectionError("viewer port closed the WebSocket")
            if b0 & 0x0F in (0, 2):
                self.payload += data
        out, self.payload = self.payload[:n], self.payload[n:]
        return out

    def send(self, data):
        mask = os.urandom(4)
        self.sock.sendall(bytes([0x82, 0x80 | len(data)]) + mask
                          + bytes(c ^ mask[i % 4] for i, c in enumerate(data)))

    def handshake(self):
        if not self.read(12).startswith(b"RFB 003."):
            raise ConnectionError("not an RFB server")
        self.send(b"RFB 003.008\n")
        if 1 not in self.read(self.read(1)[0]):
            raise ConnectionError("security type None not offered")
        self.send(b"\x01")
        if struct.unpack(">I", self.read(4))[0] != 0:
            raise ConnectionError("RFB security handshake failed")
        self.send(b"\x01")
        self.width, self.height = struct.unpack(">HH", self.read(4))
        self.read(16)
        self.read(struct.unpack(">I", self.read(4))[0])
        self.send(struct.pack(">BxxxBBBBHHHBBBxxx", 0, 32, 24, 0, 1, 255, 255, 255, 16, 8, 0))
        self.send(struct.pack(">BxHi", 2, 1, 0))
        self.fb = bytearray(self.width * self.height * 4)

    def update(self, incremental):
        self.send(struct.pack(">BBHHHH", 3, int(incremental), 0, 0, self.width, self.height))
        while True:
            kind = self.read(1)[0]
            if kind == 0:
                break
            if kind == 1:
                self.read(struct.unpack(">xHH", self.read(5))[1] * 6)
            elif kind == 3:
                self.read(3)
                self.read(struct.unpack(">I", self.read(4))[0])
            elif kind != 2:
                raise ConnectionError(f"unexpected RFB message {kind}")
        for _ in range(struct.unpack(">xH", self.read(3))[0]):
            x, y, w, h, encoding = struct.unpack(">HHHHi", self.read(12))
            if encoding != 0:
                raise ConnectionError(f"encoding {encoding} was not offered")
            data = self.read(w * h * 4)
            for row in range(h):
                start = ((y + row) * self.width + x) * 4
                self.fb[start:start + w * 4] = data[row * w * 4:(row + 1) * w * 4]

    def painted(self):
        seen = {bytes(self.fb[i:i + 3])
                for y in range(0, self.height, 37) for x in range(0, self.width, 41)
                for i in [(y * self.width + x) * 4]}
        return len(seen) >= 8


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--port", type=int, default=6080)
    parser.add_argument("--deadline", type=float, default=60)
    args = parser.parse_args()
    result, attempts, last = {"probe_start_ns": now()}, 0, ""
    while (now() - result["probe_start_ns"]) / 1e9 < args.deadline:
        attempts += 1
        try:
            viewer = Viewer(args.port)
            viewer.handshake()
            break
        except (OSError, ConnectionError) as error:
            last = str(error)
            time.sleep(0.02)
    else:
        print(json.dumps({**result, "error": "never connected", "attempts": attempts, "last": last}))
        return 1
    result.update({"attempts": attempts, "rfb_ready_ns": now(), "size": f"{viewer.width}x{viewer.height}"})
    viewer.update(incremental=False)
    result["first_update_ns"] = now()
    while not viewer.painted():
        if (now() - result["probe_start_ns"]) / 1e9 > args.deadline:
            print(json.dumps({**result, "error": "never painted"}))
            return 1
        viewer.update(incremental=True)
    result["first_painted_ns"] = now()
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
