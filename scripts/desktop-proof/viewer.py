"""The viewer side of a SOMA desktop: a byte stream to a guest port, WebSocket, and RFB 3.8.

`Tunnel` reaches a guest port through soma-api's loopback face (`GET /v1/sandboxes/{id}/tunnel/{port}`
upgraded to `soma-tunnel`); `Direct` is a plain TCP stream for checking a desktop outside SOMA.
`WebSocket` is the client half of RFC 6455 over either, binary frames only, and `Rfb` is an RFB 3.8
viewer with security type None, 32-bit true colour and Raw encoding only, so every pixel is seen,
speaking KasmVNC's dialect of the pointer event (see `Rfb.pointer`).
Only the Python standard library is used.
"""

import base64, os, select, socket, struct, time

RIGHT = 4  # RFB button mask bit for the right button
ESCAPE = 0xFF1B


def now():
    return time.time_ns()


class Tunnel:
    """A byte stream to a guest port through soma-api's loopback face."""

    def __init__(self, api, tenant, instance, port, timeout=10.0):
        host, _, api_port = api.rpartition(":")
        self.sock = socket.create_connection((host, int(api_port)), timeout=timeout)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.sock.sendall((
            f"GET /v1/sandboxes/{instance}/tunnel/{port} HTTP/1.1\r\nhost: soma\r\n"
            f"x-soma-tenant: {tenant}\r\nconnection: upgrade\r\nupgrade: soma-tunnel\r\n\r\n"
        ).encode())
        head, self.rest = self._head()
        status = head.split(b" ", 2)[1]
        if status != b"101":
            raise ConnectionError(f"tunnel refused: {head.splitlines()[0].decode()} {self.rest[:200]!r}")

    def _head(self):
        data = b""
        while b"\r\n\r\n" not in data:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise ConnectionError("tunnel closed before its answer")
            data += chunk
        head, rest = data.split(b"\r\n\r\n", 1)
        return head, rest


class WebSocket:
    """A client WebSocket over an already connected stream, binary frames only."""

    def __init__(self, tunnel):
        self.sock, self.buf, self.payload = tunnel.sock, tunnel.rest, b""
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall((
            "GET /websockify HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n"
            f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n"
            # KasmVNC closes a WebSocket upgrade that names no Origin, as a browser always does.
            "Origin: http://localhost\r\nSec-WebSocket-Protocol: binary\r\n\r\n").encode())
        while b"\r\n\r\n" not in self.buf:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise ConnectionError("viewer port closed during the WebSocket handshake")
            self.buf += chunk
        head, self.buf = self.buf.split(b"\r\n\r\n", 1)
        if b" 101 " not in head.split(b"\r\n")[0]:
            raise ConnectionError(f"WebSocket refused: {head[:200]!r}")

    def ready(self, wait_s):
        """Whether a message has started arriving within wait_s."""
        if self.payload or self.buf:
            return True
        return bool(select.select([self.sock], [], [], wait_s)[0])

    def _raw(self, n):
        while len(self.buf) < n:
            chunk = self.sock.recv(1 << 20)
            if not chunk:
                raise ConnectionError("stream ended")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def _frame(self):
        b0, b1 = self._raw(2)
        length = b1 & 0x7F
        if length == 126:
            length = struct.unpack(">H", self._raw(2))[0]
        elif length == 127:
            length = struct.unpack(">Q", self._raw(8))[0]
        mask = self._raw(4) if b1 & 0x80 else None
        data = self._raw(length)
        if mask:
            data = bytes(c ^ mask[i % 4] for i, c in enumerate(data))
        opcode = b0 & 0x0F
        if opcode == 9:
            self.send(data, opcode=10)
            return b""
        if opcode == 8:
            raise ConnectionError("viewer port closed the WebSocket")
        return data

    def read(self, n):
        while len(self.payload) < n:
            self.payload += self._frame()
        out, self.payload = self.payload[:n], self.payload[n:]
        return out

    def send(self, data, opcode=2):
        mask = os.urandom(4)
        n = len(data)
        head = bytes([0x80 | opcode])
        if n < 126:
            head += bytes([0x80 | n])
        elif n < 65536:
            head += bytes([0x80 | 126]) + struct.pack(">H", n)
        else:
            head += bytes([0x80 | 127]) + struct.pack(">Q", n)
        self.sock.sendall(head + mask + bytes(c ^ mask[i % 4] for i, c in enumerate(data)))


class Rfb:
    """RFB 3.8 with security type None, 32-bit true colour, Raw encoding."""

    def __init__(self, ws):
        self.ws = ws
        version = ws.read(12)
        if not version.startswith(b"RFB 003."):
            raise ConnectionError(f"not an RFB server: {version!r}")
        ws.send(b"RFB 003.008\n")
        types = ws.read(ws.read(1)[0])
        if 1 not in types:
            raise ConnectionError(f"security type None not offered: {list(types)}")
        ws.send(b"\x01")
        if struct.unpack(">I", ws.read(4))[0] != 0:
            raise ConnectionError("RFB security handshake failed")
        ws.send(b"\x01")  # shared
        self.width, self.height = struct.unpack(">HH", ws.read(4))
        ws.read(16)
        self.name = ws.read(struct.unpack(">I", ws.read(4))[0]).decode(errors="replace")
        ws.send(struct.pack(">BxxxBBBBHHHBBBxxx", 0, 32, 24, 0, 1, 255, 255, 255, 16, 8, 0))
        ws.send(struct.pack(">BxHi", 2, 1, 0))
        self.fb = bytearray(self.width * self.height * 4)

    def request(self, incremental=True):
        self.ws.send(struct.pack(">BBHHHH", 3, int(incremental), 0, 0, self.width, self.height))

    def update(self, region=None, wait_s=None):
        """Reads server messages up to one framebuffer update; returns pixels changed in region.

        With wait_s, returns None when no message starts within it. A message is only ever
        waited for before its first byte, so a timeout can never leave the stream half read.
        """
        if wait_s is not None and not self.ws.ready(wait_s):
            return None
        while True:
            kind = self.ws.read(1)[0]
            if kind == 0:
                break
            if kind == 2:
                continue
            if kind == 1:
                self.ws.read(struct.unpack(">xHH", self.ws.read(5))[1] * 6)
                continue
            if kind == 3:
                self.ws.read(3)
                self.ws.read(struct.unpack(">I", self.ws.read(4))[0])
                continue
            raise ConnectionError(f"unexpected RFB message {kind}")
        changed = 0
        for _ in range(struct.unpack(">xH", self.ws.read(3))[0]):
            x, y, w, h, encoding = struct.unpack(">HHHHi", self.ws.read(12))
            if encoding != 0:
                raise ConnectionError(f"server sent encoding {encoding}, only Raw was offered")
            data = self.ws.read(w * h * 4)
            for row in range(h):
                start = ((y + row) * self.width + x) * 4
                line = data[row * w * 4:(row + 1) * w * 4]
                if region and region[1] <= y + row < region[3]:
                    lo, hi = max(x, region[0]), min(x + w, region[2])
                    if lo < hi:
                        old = self.fb[start + (lo - x) * 4:start + (hi - x) * 4]
                        new = line[(lo - x) * 4:(hi - x) * 4]
                        changed += sum(old[i:i + 4] != new[i:i + 4] for i in range(0, len(new), 4))
                self.fb[start:start + w * 4] = line
        return changed

    def painted(self):
        """True when the frame is not one flat colour: at least 8 distinct values on a grid."""
        seen = set()
        for y in range(0, self.height, 37):
            for x in range(0, self.width, 41):
                i = (y * self.width + x) * 4
                seen.add(bytes(self.fb[i:i + 3]))
        return len(seen) >= 8

    def pointer(self, x, y, mask):
        # KasmVNC's PointerEvent is 11 bytes, not RFB's 6: a 16-bit button mask and two signed
        # 16-bit scroll deltas. A standard 6-byte event desynchronises the server, which then
        # reads the next request as a SetPixelFormat and drops the viewer ("invalid pixel format").
        self.ws.send(struct.pack(">BHHHhh", 5, mask, x, y, 0, 0))

    def key(self, keysym):
        self.ws.send(struct.pack(">BBxxI", 4, 1, keysym) + struct.pack(">BBxxI", 4, 0, keysym))

    def settle(self, quiet_ms=300, limit_s=5.0):
        """Requests updates until none changes the frame for quiet_ms."""
        deadline, last = time.monotonic() + limit_s, time.monotonic()
        while time.monotonic() < deadline:
            self.request()
            if self.update((0, 0, self.width, self.height), wait_s=quiet_ms / 1000):
                last = time.monotonic()
            if time.monotonic() - last >= quiet_ms / 1000:
                return

    def until_change(self, region, minimum, limit_s=5.0):
        """Requests updates until at least `minimum` pixels in region change; returns ns or None."""
        deadline = time.monotonic() + limit_s
        while time.monotonic() < deadline:
            self.request()
            if self.update(region) >= minimum:
                return now()
        return None


class Direct:
    """A plain TCP stream to host:port, for checking a desktop outside SOMA."""

    def __init__(self, address, timeout=10.0):
        host, _, port = address.rpartition(":")
        self.sock = socket.create_connection((host, int(port)), timeout=timeout)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.rest = b""
