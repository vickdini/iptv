"""Generates packaging/windows/iptv.ico (a stylised TV). Requires Pillow.

Usage: python make-icon.py
"""
from pathlib import Path

from PIL import Image, ImageDraw

S = 1024  # draw large, downsample for crisp small sizes


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def draw() -> Image.Image:
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # Antennas.
    w = 44
    d.line([(512, 300), (330, 90)], fill=(60, 66, 80, 255), width=w)
    d.line([(512, 300), (700, 110)], fill=(60, 66, 80, 255), width=w)
    for x, y in [(330, 90), (700, 110)]:
        d.ellipse([x - 42, y - 42, x + 42, y + 42], fill=(255, 176, 32, 255))

    # Body.
    d.rounded_rectangle([60, 260, 964, 900], radius=110, fill=(34, 38, 48, 255))

    # Screen with a vertical gradient.
    screen = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    grad = Image.new("RGBA", (S, S))
    top, bottom = (64, 140, 255), (124, 58, 237)
    gd = ImageDraw.Draw(grad)
    for y in range(S):
        gd.line([(0, y), (S, y)], fill=lerp(top, bottom, y / S) + (255,))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([130, 330, 894, 830], radius=70, fill=255)
    screen.paste(grad, (0, 0), mask)
    img.alpha_composite(screen)

    # Play symbol.
    d = ImageDraw.Draw(img)
    d.polygon([(430, 450), (430, 710), (650, 580)], fill=(255, 255, 255, 240))

    # Feet.
    d.rounded_rectangle([230, 890, 330, 960], radius=20, fill=(34, 38, 48, 255))
    d.rounded_rectangle([694, 890, 794, 960], radius=20, fill=(34, 38, 48, 255))
    return img


if __name__ == "__main__":
    out = Path(__file__).with_name("iptv.ico")
    big = draw().resize((256, 256), Image.LANCZOS)
    big.save(out, sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    big.save(out.with_suffix(".png"))
    print(f"wrote {out}")
