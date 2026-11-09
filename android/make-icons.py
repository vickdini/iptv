"""Generates the Android launcher icons and the Android TV banner from the
same drawing as the Windows icon (packaging/windows/make-icon.py). Requires Pillow.

Usage: python android/make-icons.py
"""
import importlib.util
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

here = Path(__file__).parent
spec = importlib.util.spec_from_file_location("make_icon", here.parent / "packaging/windows/make-icon.py")
make_icon = importlib.util.module_from_spec(spec)
spec.loader.exec_module(make_icon)

res = here / "app/src/main/res"
tv = make_icon.draw()

# Launcher icons on a dark rounded square, per screen density.
for density, size in {"mdpi": 48, "hdpi": 72, "xhdpi": 96, "xxhdpi": 144, "xxxhdpi": 192}.items():
    big = 1024
    bg = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    ImageDraw.Draw(bg).rounded_rectangle([0, 0, big - 1, big - 1], radius=220, fill=(18, 20, 28, 255))
    inner = tv.resize((820, 820), Image.LANCZOS)
    bg.alpha_composite(inner, (102, 90))
    out = res / f"mipmap-{density}" / "ic_launcher.png"
    out.parent.mkdir(parents=True, exist_ok=True)
    bg.resize((size, size), Image.LANCZOS).save(out)

# Android TV home-screen banner (320x180 at xhdpi).
w, h = 1280, 720
banner = Image.new("RGBA", (w, h), (18, 20, 28, 255))
banner.alpha_composite(tv.resize((560, 560), Image.LANCZOS), (70, 80))
d = ImageDraw.Draw(banner)
try:
    font = ImageFont.truetype("arialbd.ttf", 230)
except OSError:
    font = ImageFont.load_default()
d.text((680, 360), "IPTV", font=font, fill=(255, 255, 255, 255), anchor="lm")
out = res / "drawable-xhdpi" / "banner.png"
banner.resize((320, 180), Image.LANCZOS).save(out)
print("icons written")
