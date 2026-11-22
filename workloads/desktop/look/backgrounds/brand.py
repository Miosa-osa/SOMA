"""The admin chat's NASA backdrops as desktop wallpapers, each with a small MIOSA wordmark."""
import glob, os, sys
from PIL import Image
SRC, LOGO, OUT = sys.argv[1], sys.argv[2], sys.argv[3]
os.makedirs(OUT, exist_ok=True)
for f in sorted(glob.glob(f"{SRC}/*-2560.webp")):
    name = os.path.basename(f).replace("-2560.webp", "")
    im = Image.open(f).convert("RGB")
    if im.width != 2560:
        im = im.resize((2560, int(im.height * 2560 / im.width)), Image.LANCZOS)
    light = "-light" in name
    logo = Image.open(LOGO).convert("LA")
    w = int(im.width * 0.15)
    logo = logo.resize((w, int(logo.height * w / logo.width)), Image.LANCZOS)
    color = (16, 22, 60) if light else (240, 243, 255)
    a = logo.getchannel("A").point(lambda p: int(p * 0.9))
    im.paste(Image.new("RGB", logo.size, color), ((im.width - w) // 2, (im.height - logo.height) // 2), a)
    im.save(f"{OUT}/miosa-{name}.jpg", quality=92, subsampling=0)
    print(name, im.size)
