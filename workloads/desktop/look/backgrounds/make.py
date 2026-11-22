"""MIOSA wallpapers, dark and light: layered silk waves (macOS style) under a centred MIOSA wordmark."""
import math, sys
from PIL import Image, ImageDraw, ImageFilter, ImageChops

W, H = 2560, 1600
LOGO, OUT = sys.argv[1], sys.argv[2]

def vertical(top, bottom):
    g = Image.new("RGB", (1, H))
    for y in range(H):
        t = y / (H - 1)
        g.putpixel((0, y), tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3)))
    return g.resize((W, H))

def wave(im, base_y, amp, freq, phase, tilt, c_top, c_bottom, alpha, blur):
    mask = Image.new("L", (W, H), 0)
    pts = []
    for x in range(0, W + 8, 8):
        u = x / W
        y = base_y * H + amp * H * math.sin(2 * math.pi * (freq * u) + phase) + tilt * H * (u - 0.5)
        pts.append((x, y))
    pts += [(W, H), (0, H)]
    ImageDraw.Draw(mask).polygon(pts, fill=alpha)
    mask = mask.filter(ImageFilter.GaussianBlur(blur))
    im.paste(vertical(c_top, c_bottom), (0, 0), mask)
    # a fine bright edge along the crest, the "silk" highlight
    edge = Image.new("L", (W, H), 0)
    ImageDraw.Draw(edge).line(pts[:-2], fill=int(alpha * 0.55), width=3)
    edge = edge.filter(ImageFilter.GaussianBlur(4))
    im.paste(Image.new("RGB", (W, H), tuple(min(255, c + 90) for c in c_top)), (0, 0), edge)

def glow(im, cx, cy, r, color, alpha):
    m = Image.new("L", (W, H), 0)
    ImageDraw.Draw(m).ellipse([cx * W - r * W, cy * H - r * W, cx * W + r * W, cy * H + r * W], fill=alpha)
    im.paste(Image.new("RGB", (W, H), color), (0, 0), m.filter(ImageFilter.GaussianBlur(220)))

def wordmark(im, color, opacity):
    logo = Image.open(LOGO).convert("LA")
    width = int(W * 0.24)
    logo = logo.resize((width, int(logo.height * width / logo.width)), Image.LANCZOS)
    a = logo.getchannel("A").point(lambda v: int(v * opacity))
    x, y = (W - logo.width) // 2, int(H * 0.40) - logo.height // 2
    im.paste(Image.new("RGB", logo.size, color), (x, y), a)

def grain(im, k):
    n = Image.effect_noise((W, H), 48).convert("RGB").point(lambda v: int(v * k))
    return ImageChops.add(im, n)

# Dark: midnight sky, indigo and violet silk rising from the lower right.
d = vertical((5, 6, 14), (10, 9, 26))
glow(d, 0.5, 0.40, 0.22, (60, 56, 170), 110)
wave(d, 0.70, 0.07, 0.9, 0.6, -0.20, (34, 30, 110), (8, 8, 24), 235, 6)
wave(d, 0.78, 0.06, 1.1, 2.1, -0.28, (82, 52, 190), (14, 10, 40), 230, 5)
wave(d, 0.88, 0.05, 0.8, 4.0, -0.22, (40, 110, 210), (8, 14, 36), 225, 5)
wave(d, 0.96, 0.04, 1.3, 1.2, -0.15, (120, 60, 200), (12, 8, 30), 210, 6)
d = grain(d, 0.035)
wordmark(d, (244, 245, 255), 0.95)
d.save(f"{OUT}/miosa-dark.jpg", quality=93, subsampling=0)

# Light: morning haze, lilac and sky silk.
l = vertical((240, 241, 250), (226, 230, 246))
glow(l, 0.5, 0.40, 0.24, (255, 255, 255), 160)
wave(l, 0.70, 0.07, 0.9, 0.6, -0.20, (205, 200, 250), (236, 236, 250), 235, 6)
wave(l, 0.78, 0.06, 1.1, 2.1, -0.28, (180, 170, 245), (225, 222, 248), 230, 5)
wave(l, 0.88, 0.05, 0.8, 4.0, -0.22, (160, 205, 250), (220, 232, 250), 225, 5)
wave(l, 0.96, 0.04, 1.3, 1.2, -0.15, (235, 185, 230), (245, 225, 242), 210, 6)
l = grain(l, 0.03)
wordmark(l, (18, 20, 34), 0.9)
l.save(f"{OUT}/miosa-light.jpg", quality=93, subsampling=0)
print("ok")
