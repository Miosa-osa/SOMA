"""Night City: generated grayscale scenes of a city at night (layered skyline, lit windows, haze, rain,
a glow on the horizon), coloured later by duotone.py in the "haze" or "neon" palette. Original work.

    python3 night_city.py OUT_DIR
"""
import os
import sys

import numpy as np
from PIL import Image, ImageFilter

W, H = 2560, 1440
OUT = sys.argv[1]
os.makedirs(OUT, exist_ok=True)


def skyline(rng, base, height, width_range, density, tower_chance=0.08):
    """A row of buildings: returns a height per column (in rows from the top)."""
    tops = np.full(W, H, np.float32)
    x = 0
    while x < W:
        w = int(rng.integers(*width_range))
        h = height * (0.35 + 0.65 * rng.random() ** 1.6)
        if rng.random() < tower_chance:
            h *= 1.7
        top = base - h
        tops[x:x + w] = np.minimum(tops[x:x + w], top)
        if rng.random() < 0.25:  # a spire or antenna
            mid = x + w // 2
            tops[mid:mid + 3] = np.minimum(tops[mid:mid + 3], top - h * 0.25)
        x += w + int(rng.integers(0, density))
    return tops


def scene(seed, glow_x, glow_y, haze, rain, layers):
    rng = np.random.default_rng(seed)
    y, x = np.mgrid[0:H, 0:W].astype(np.float32)
    u, v = x / W, y / H
    # Sky: dark at the top, a wide glow behind the city (light pollution, a low sun through smog).
    img = 0.05 + 0.10 * v
    r = np.sqrt(((u - glow_x) * 1.6) ** 2 + (v - glow_y) ** 2)
    img += 0.85 * np.exp(-(r / 0.28) ** 2) + 0.30 * np.exp(-(r / 0.7) ** 2)
    for i, (base, height, wr, dens) in enumerate(layers):
        tops = skyline(rng, base * H, height * H, wr, dens)
        mask = y >= tops[None, :]
        depth = i / max(len(layers) - 1, 1)  # 0 far, 1 near
        # Far buildings take the haze colour; near ones are near-black silhouettes.
        body = (0.42 - 0.40 * depth) * (0.6 + 0.4 * np.exp(-(r / 0.5) ** 2))
        img = np.where(mask, body, img)
        # Lit windows: a sparse grid on each layer, brighter on the far layers.
        win = (rng.random((H // 6 + 1, W // 4 + 1)) < (0.05 + 0.05 * (1 - depth))).repeat(6, 0).repeat(4, 1)[:H, :W]
        grid = ((x % 4) < 2) & ((y % 6) < 3)
        img = np.where(mask & win & grid & (y > tops[None, :] + 8), 0.55 + 0.35 * (1 - depth), img)
        # Haze between the layers.
        img = img * (1 - haze * (1 - depth) * 0.35) + haze * 0.18 * (1 - depth)
    if rain:
        drops = rng.random((H, W)) < 0.0016
        streak = Image.fromarray((drops * 255).astype(np.uint8)).filter(ImageFilter.BoxBlur(0))
        k = np.zeros((25, 25)); np.fill_diagonal(np.fliplr(k)[:, ::-1], 1)
        streak = np.asarray(streak, np.float32) / 255
        streak = np.asarray(Image.fromarray((streak * 255).astype(np.uint8)).filter(ImageFilter.Kernel((5, 5), [0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0], scale=1)), np.float32) / 255
        img = np.maximum(img, streak * 0.45)
    img = np.clip(img, 0, 1)
    return Image.fromarray((img * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(0.6))


SCENES = {
    # name: (seed, glow x, glow y, haze, rain, layers: (base, height, building widths, gap))
    "night-city": (3, 0.62, 0.58, 0.9, False, [(0.70, 0.30, (30, 90), 30), (0.82, 0.42, (40, 120), 20), (1.0, 0.55, (60, 170), 12)]),
    "rain-city": (11, 0.35, 0.62, 0.6, True, [(0.72, 0.26, (24, 70), 24), (0.86, 0.40, (40, 110), 16), (1.0, 0.62, (70, 200), 10)]),
    "megatowers": (7, 0.5, 0.52, 1.0, False, [(0.66, 0.46, (60, 150), 40), (0.84, 0.60, (80, 220), 30), (1.0, 0.70, (120, 300), 20)]),
}
for name, (seed, gx, gy, haze, rain, layers) in SCENES.items():
    scene(seed, gx, gy, haze, rain, layers).save(os.path.join(OUT, f"{name}.webp"), quality=80, method=6)
    print(name)
