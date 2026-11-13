"""Рисует логотип CRM и иконки трея (оригинальный рисунок: квадрат с галочкой).

Запуск: python3 scripts/make_icons.py crates/crm-desktop/icons
"""
import sys
from pathlib import Path
from PIL import Image, ImageDraw

INDIGO_TOP = (99, 102, 241)
INDIGO_BOTTOM = (67, 56, 202)
DOTS = {"ok": (34, 197, 94), "warn": (234, 179, 8), "error": (239, 68, 68), "offline": (148, 163, 184)}


def logo(size: int) -> Image.Image:
    s = size * 4  # рисуем крупно и уменьшаем — гладкие края
    grad = Image.new("RGB", (s, s))
    d = ImageDraw.Draw(grad)
    for y in range(s):
        t = y / s
        d.line([(0, y), (s, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(INDIGO_TOP, INDIGO_BOTTOM)))
    mask = Image.new("L", (s, s), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, s - 1, s - 1], radius=int(s * 0.22), fill=255)
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    img.paste(grad, (0, 0), mask)
    w = max(4, int(s * 0.11))
    ImageDraw.Draw(img).line(
        [(s * 0.27, s * 0.52), (s * 0.44, s * 0.68), (s * 0.74, s * 0.34)],
        fill=(255, 255, 255, 255), width=w, joint="curve",
    )
    return img.resize((size, size), Image.LANCZOS)


def tray(state: str, size: int = 64) -> Image.Image:
    img = logo(size)
    s = size * 4
    big = img.resize((s, s), Image.LANCZOS)
    d = ImageDraw.Draw(big)
    r = int(s * 0.21)
    cx, cy = s - r - 2, s - r - 2
    d.ellipse([cx - r - 10, cy - r - 10, cx + r + 10, cy + r + 10], fill=(255, 255, 255, 255))
    d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=DOTS[state] + (255,))
    return big.resize((size, size), Image.LANCZOS)


if __name__ == "__main__":
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    logo(1024).save(out / "source.png")
    for state in DOTS:
        tray(state).save(out / f"tray-{state}.png")
    print("готово:", out)
