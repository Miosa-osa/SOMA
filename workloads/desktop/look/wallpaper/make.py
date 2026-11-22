"""Writes the SOMA desktop wallpaper: a deep navy field with a soft violet glow, as a PNG.

Pure Python (zlib only), so the image needs no imaging package. Run once at image build.
"""

import math
import struct
import sys
import zlib

WIDTH, HEIGHT = 1920, 1080
BASE = (9, 11, 18)
GLOWS = [  # (x, y, radius, color, strength) in screen fractions
    (0.78, 0.18, 0.75, (91, 91, 240), 0.34),
    (0.12, 0.92, 0.65, (40, 120, 200), 0.22),
]


def pixel(x, y):
    r, g, b = BASE
    for gx, gy, radius, color, strength in GLOWS:
        dx = (x / WIDTH - gx) * (WIDTH / HEIGHT)
        dy = y / HEIGHT - gy
        falloff = max(0.0, 1.0 - math.hypot(dx, dy) / radius)
        weight = strength * falloff * falloff
        r += (color[0] - r) * weight
        g += (color[1] - g) * weight
        b += (color[2] - b) * weight
    # A little ordered noise keeps the gradient from banding in the H.264 stream.
    n = ((x * 7 + y * 13) % 5) - 2
    return bytes((max(0, min(255, int(r) + n)), max(0, min(255, int(g) + n)), max(0, min(255, int(b) + n))))


def chunk(kind, data):
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


def main(path):
    rows = bytearray()
    for y in range(HEIGHT):
        rows.append(0)
        rows.extend(b"".join(pixel(x, y) for x in range(WIDTH)))
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", WIDTH, HEIGHT, 8, 2, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(rows), 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as out:
        out.write(png)


if __name__ == "__main__":
    main(sys.argv[1])
