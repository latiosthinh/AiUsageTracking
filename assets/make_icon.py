"""Generate deterministic app icon assets (no network). Run: python -I assets/make_icon.py"""
from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent
SIZE = 256
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)

BG = (0x14, 0x14, 0x18, 255)
BORDER = (0x37, 0x37, 0x3C, 255)
TRACK = (0x3C, 0x3C, 0x3C, 255)
GREEN = (0x4C, 0xAF, 0x50, 255)
AMBER = (0xFF, 0xB3, 0x00, 255)

BAR_LEFT = 44
BAR_RIGHT = 212
BAR_WIDTH = BAR_RIGHT - BAR_LEFT
BAR_HEIGHT = 34
BAR_RADIUS = BAR_HEIGHT // 2
TOP_Y = 84
BOTTOM_Y = 148
SQUARE_RADIUS = 48
BORDER_PX = 6


def draw_icon(size: int) -> Image.Image:
    scale = size / SIZE
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    def s(v: float) -> int:
        return max(1, round(v * scale))

    outer = [0, 0, size - 1, size - 1]
    draw.rounded_rectangle(outer, radius=s(SQUARE_RADIUS), fill=BG)
    inset = s(BORDER_PX)
    inner = [inset, inset, size - 1 - inset, size - 1 - inset]
    draw.rounded_rectangle(
        inner,
        radius=max(1, s(SQUARE_RADIUS - BORDER_PX)),
        outline=BORDER,
        width=max(1, s(BORDER_PX)),
    )

    bar_w = max(1, round(BAR_WIDTH * scale))
    bar_h = max(1, round(BAR_HEIGHT * scale))
    bar_r = max(1, round(BAR_RADIUS * scale))
    left = round(BAR_LEFT * scale)

    def bar(y_design: int, fraction: float, fill: tuple[int, int, int, int]) -> None:
        y = round(y_design * scale)
        track_box = [left, y, left + bar_w, y + bar_h]
        draw.rounded_rectangle(track_box, radius=bar_r, fill=TRACK)
        fill_w = max(bar_r * 2, round(bar_w * fraction))
        fill_box = [left, y, left + fill_w, y + bar_h]
        draw.rounded_rectangle(fill_box, radius=bar_r, fill=fill)

    bar(TOP_Y, 0.65, GREEN)
    bar(BOTTOM_Y, 0.35, AMBER)
    return img


def main() -> None:
    master = draw_icon(SIZE)
    master.save(ROOT / "icon.png", format="PNG")

    ico_images = [draw_icon(sz) for sz in ICO_SIZES]
    ico_images[0].save(
        ROOT / "icon.ico",
        format="ICO",
        sizes=[(sz, sz) for sz in ICO_SIZES],
        append_images=ico_images[1:],
    )


if __name__ == "__main__":
    main()
