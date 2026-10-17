"""Renders assets/nib.ico: the same pen-nib design the app draws at runtime
(gfx::make_icon), at every size Windows asks for. Run: python assets/make_icon.py"""
from pathlib import Path
from PIL import Image, ImageDraw

BRAND = (15, 118, 110, 255)  # #0f766e
WHITE = (255, 255, 255, 255)
SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]


def render(size: int) -> Image.Image:
    ss = 8  # supersample, then downscale for smooth edges
    s = size * ss
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([0, 0, s - 1, s - 1], radius=int(s * 0.22), fill=BRAND)
    # A kite-shaped nib pointing down-left.
    d.polygon([(s * 0.20, s * 0.82), (s * 0.38, s * 0.38), (s * 0.62, s * 0.18), (s * 0.82, s * 0.38), (s * 0.62, s * 0.62)], fill=WHITE)
    # Slit and breather hole.
    d.line([(s * 0.21, s * 0.81), (s * 0.53, s * 0.49)], fill=BRAND, width=max(1, int(s * 0.06)))
    r = s * 0.06
    cx, cy = s * 0.57, s * 0.43
    d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=BRAND)
    return img.resize((size, size), Image.LANCZOS)


out = Path(__file__).with_name("nib.ico")
images = [render(n) for n in SIZES]
images[-1].save(out, format="ICO", sizes=[(n, n) for n in SIZES], append_images=images[:-1])
print(f"wrote {out} ({out.stat().st_size} bytes)")
