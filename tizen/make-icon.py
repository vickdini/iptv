"""Generates the Samsung TV app icon (tizen/app/icon.png, 512x423 as Samsung
recommends) from the shared TV drawing in packaging/windows/make-icon.py."""
import importlib.util
from pathlib import Path

from PIL import Image, ImageDraw

here = Path(__file__).parent
spec = importlib.util.spec_from_file_location("make_icon", here.parent / "packaging/windows/make-icon.py")
make_icon = importlib.util.module_from_spec(spec)
spec.loader.exec_module(make_icon)

w, h = 512, 423
img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
ImageDraw.Draw(img).rounded_rectangle([0, 0, w - 1, h - 1], radius=48, fill=(18, 20, 28, 255))
img.alpha_composite(make_icon.draw().resize((360, 360), Image.LANCZOS), ((w - 360) // 2, (h - 360) // 2 + 4))
img.save(here / "app" / "icon.png")
print("wrote tizen/app/icon.png")
