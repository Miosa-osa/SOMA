"""MIOSA duotone: a photo in the platform's blue (the ramp measured from the admin Aurora backdrop),
with a fine ordered-dither grain, the MIOSA wordmark at the centre, dark and light, 2560 x 1440."""
import glob, os, sys
import numpy as np
from PIL import Image, ImageOps

SRC, LOGO, OUT = sys.argv[1], sys.argv[2], sys.argv[3]
os.makedirs(OUT, exist_ok=True)
W, H = 2560, 1440
# gray level -> colour, from aurora-gray.webp against aurora-2560 / aurora-light-2560
DARK = [(0, (6, 9, 26)), (50, (15, 23, 64)), (100, (30, 45, 113)), (150, (57, 79, 181)), (230, (138, 157, 242)), (255, (205, 214, 255))]
LIGHT = [(0, (83, 106, 210)), (50, (139, 159, 239)), (100, (180, 193, 249)), (150, (209, 217, 255)), (230, (237, 241, 255)), (255, (248, 250, 255))]

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
    dark = render(gray, DARK, (240, 243, 255), f"{OUT}/miosa-{name}.jpg")
    render(gray, LIGHT, (16, 22, 60), f"{OUT}/miosa-{name}-light.jpg")
    thumbs.append((name, dark.resize((640, 360), Image.LANCZOS)))
cols = 3
sheet = Image.new("RGB", (640 * cols, 360 * ((len(thumbs) + cols - 1) // cols)))
for i, (_, t) in enumerate(thumbs):
    sheet.paste(t, ((i % cols) * 640, (i // cols) * 360))
sheet.save(f"{OUT}/sheet.jpg", quality=85)
print([n for n, _ in thumbs])
