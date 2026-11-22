"""MIOSA duotone: a photo in the platform's blue (the ramp measured from the admin Aurora backdrop),
with a fine ordered-dither grain, the MIOSA wordmark at the centre, dark and light, 2560 x 1440."""
import glob, os, sys
import numpy as np
from PIL import Image, ImageOps

SRC, LOGO, OUT = sys.argv[1], sys.argv[2], sys.argv[3]
os.makedirs(OUT, exist_ok=True)
W, H = 2560, 1440
# gray level -> colour. "blue" is measured from aurora-gray.webp against aurora-2560 /
# aurora-light-2560; the others follow the same curve in their own hue.
BLUE_DARK = [(0, (6, 9, 26)), (50, (15, 23, 64)), (100, (30, 45, 113)), (150, (57, 79, 181)), (230, (138, 157, 242)), (255, (205, 214, 255))]
BLUE_LIGHT = [(0, (83, 106, 210)), (50, (139, 159, 239)), (100, (180, 193, 249)), (150, (209, 217, 255)), (230, (237, 241, 255)), (255, (248, 250, 255))]

PALETTES = {
    "blue": (BLUE_DARK, BLUE_LIGHT),
    "amber": ([(0, (8, 5, 4)), (50, (42, 17, 8)), (100, (110, 42, 14)), (150, (196, 88, 30)), (230, (255, 170, 92)), (255, (255, 226, 194))],
              [(0, (150, 72, 32)), (50, (205, 118, 62)), (100, (236, 164, 108)), (150, (248, 198, 150)), (230, (255, 232, 206)), (255, (255, 246, 236))]),
    "ink": ([(0, (5, 9, 16)), (50, (13, 29, 50)), (100, (28, 66, 104)), (150, (64, 124, 164)), (230, (176, 212, 228)), (255, (238, 246, 250))],
            [(0, (40, 84, 118)), (50, (88, 134, 166)), (100, (140, 178, 202)), (150, (182, 210, 226)), (230, (226, 240, 246)), (255, (246, 251, 253))]),
    "stone": ([(0, (9, 7, 5)), (50, (38, 28, 18)), (100, (92, 70, 44)), (150, (160, 128, 86)), (230, (226, 204, 160)), (255, (250, 240, 220))],
              [(0, (122, 96, 64)), (50, (164, 136, 98)), (100, (198, 176, 138)), (150, (222, 206, 174)), (230, (242, 234, 214)), (255, (251, 247, 238))]),
    "haze": ([(0, (6, 4, 3)), (50, (34, 16, 8)), (100, (96, 42, 14)), (150, (186, 92, 30)), (230, (255, 168, 80)), (255, (255, 222, 180))],
             [(0, (128, 66, 30)), (50, (182, 108, 54)), (100, (222, 150, 92)), (150, (240, 186, 136)), (230, (252, 226, 196)), (255, (255, 244, 232))]),
    "neon": ([(0, (5, 4, 14)), (50, (28, 10, 52)), (100, (96, 20, 120)), (150, (200, 40, 160)), (200, (60, 200, 230)), (255, (210, 250, 255))],
             [(0, (88, 52, 140)), (50, (140, 92, 190)), (100, (200, 130, 210)), (150, (236, 170, 220)), (200, (170, 232, 246)), (255, (240, 252, 255))]),
    "forest": ([(0, (4, 9, 6)), (50, (12, 28, 18)), (100, (30, 64, 40)), (150, (68, 118, 78)), (230, (168, 208, 168)), (255, (234, 245, 232))],
               [(0, (52, 96, 64)), (50, (98, 142, 104)), (100, (146, 184, 148)), (150, (186, 214, 184)), (230, (228, 242, 226)), (255, (246, 251, 245))]),
}
# Which palette each source takes, by file name; anything not listed is MIOSA blue.
PALETTE_OF = {}
_pal = os.path.join(SRC, "palettes.txt")
if os.path.exists(_pal):
    for line in open(_pal):
        parts = line.split()
        if len(parts) == 2 and not line.startswith("#"):
            PALETTE_OF[parts[0]] = parts[1]


def lut(points):
    xs = [p[0] for p in points]
    return np.stack([np.interp(np.arange(256), xs, [p[1][c] for p in points]) for c in range(3)], 1)

BAYER = np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]], np.float32) / 16 - 0.47

def render(gray, points, logo_rgb, path):
    h, w = gray.shape
    yy, xx = np.mgrid[0:h, 0:w]
    d = BAYER[(yy // 2) % 4, (xx // 2) % 4]          # 2 px cells, like the admin grain
    g = np.clip(gray + d * 34, 0, 255)
    g = (np.round(g / 255 * 9) / 9 * 255).astype(np.uint8)   # 10 tone steps, which makes the dots
    img = lut(points)[g].astype(np.uint8)
    im = Image.fromarray(img)
    logo = Image.open(LOGO).convert("LA")
    lw = int(W * 0.15)
    logo = logo.resize((lw, int(logo.height * lw / logo.width)), Image.LANCZOS)
    a = logo.getchannel("A").point(lambda p: int(p * 0.92))
    im.paste(Image.new("RGB", logo.size, logo_rgb), ((W - lw) // 2, (H - logo.height) // 2), a)
    im.save(path, quality=90, subsampling=0)
    return im

thumbs = []
for f in sorted(glob.glob(f"{SRC}/*.jpg") + glob.glob(f"{SRC}/*.webp")):
    name = os.path.splitext(os.path.basename(f))[0]
    src = ImageOps.exif_transpose(Image.open(f)).convert("L")
    src = ImageOps.fit(src, (W, H), Image.LANCZOS)
    src = ImageOps.autocontrast(src, cutoff=0.5)
    gray = np.asarray(src, np.float32)
    dark_pts, light_pts = PALETTES[PALETTE_OF.get(name, "blue")]
    dark = render(gray, dark_pts, (245, 245, 250), f"{OUT}/miosa-{name}.jpg")
    render(gray, light_pts, (24, 20, 30), f"{OUT}/miosa-{name}-light.jpg")
    thumbs.append((name, dark.resize((640, 360), Image.LANCZOS)))
cols = 3
sheet = Image.new("RGB", (640 * cols, 360 * ((len(thumbs) + cols - 1) // cols)))
for i, (_, t) in enumerate(thumbs):
    sheet.paste(t, ((i % cols) * 640, (i // cols) * 360))
sheet.save(f"{OUT}/sheet.jpg", quality=85)
print([n for n, _ in thumbs])
