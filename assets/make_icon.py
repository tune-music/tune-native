# Значок Tune Native: значок Tune Client (ползунки на тёмном квадрате) в его цветах, только
# ручки ползунков — белые кружки вместо черт.
# Запуск: python make_icon.py — рядом появятся tune.ico и tune.png (256×256).
from pathlib import Path
from PIL import Image, ImageDraw

OUT = Path(__file__).parent
S = 4  # рисуем крупнее и уменьшаем: края сглаживаются
BG, TRACK, KNOB = "#18181b", "#facc15", "#fafafa"


def draw(size):
    big = size * S
    k = big / 512
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((0, 0, big - 1, big - 1), radius=112 * k, fill=BG)
    w = 36 * k
    for x, knob in ((136, 245), (256, 205), (376, 285)):
        d.line(((x * k, 112 * k), (x * k, 400 * k)), fill=TRACK, width=round(w))
        for y in (112, 400):
            d.ellipse((x * k - w / 2, y * k - w / 2, x * k + w / 2, y * k + w / 2), fill=TRACK)
        r = 40 * k
        d.ellipse((x * k - r - 14 * k, knob * k - r - 14 * k, x * k + r + 14 * k, knob * k + r + 14 * k), fill=BG)
        d.ellipse((x * k - r, knob * k - r, x * k + r, knob * k + r), fill=KNOB)
    return img.resize((size, size), Image.LANCZOS)


sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
images = [draw(s) for s in sizes]
images[-1].save(OUT / "tune.png")
images[-1].save(OUT / "tune.ico", sizes=[(s, s) for s in sizes], append_images=images[:-1])
