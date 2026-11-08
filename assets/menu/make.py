# Иконки меню трея: маски прозрачности (по байту на пиксель) для размеров 16/20/24/32.
# Цвет подставляет Tune под тему Windows (main.rs, menu_icon).
# Запуск: python make.py — файлы <имя>_<размер>.a8 появятся рядом.
from pathlib import Path
from PIL import Image, ImageDraw

OUT = Path(__file__).parent
SIZES = (16, 20, 24, 32)
S = 8  # рисуем в 8 раз крупнее и уменьшаем — так края сглаживаются
G = 24  # сетка 24×24, как у иконок окна Tune


def canvas(size):
    img = Image.new("L", (size * S, size * S), 0)
    return img, ImageDraw.Draw(img), size * S / G


def line(d, k, pts, w=1.9):
    pts = [(x * k, y * k) for x, y in pts]
    d.line(pts, fill=255, width=round(w * k), joint="curve")
    r = w * k / 2
    for x, y in (pts[0], pts[-1]):
        d.ellipse((x - r, y - r, x + r, y + r), fill=255)


def arc(d, k, box, start, end, w=1.9):
    d.arc([v * k for v in box], start, end, fill=255, width=round(w * k))


def poly(d, k, pts):
    d.polygon([(x * k, y * k) for x, y in pts], fill=255)


def music(d, k):
    line(d, k, [(9, 18), (9, 5), (19, 3), (19, 16)])
    d.ellipse([v * k for v in (3.5, 15, 9.5, 21)], fill=255)
    d.ellipse([v * k for v in (13.5, 13, 19.5, 19)], fill=255)


def sliders(d, k):
    for x, y1, y2, knob in ((5, 3, 21, 14), (12, 3, 21, 8), (19, 3, 21, 16)):
        line(d, k, [(x, y1), (x, knob - 2.2)])
        line(d, k, [(x, knob + 2.2), (x, y2)])
        line(d, k, [(x - 2.6, knob), (x + 2.6, knob)])


def reload(d, k):
    arc(d, k, (4, 4, 20, 20), -60, 250)
    poly(d, k, [(20.5, 2.5), (20.5, 9.5), (13.5, 9.5)])


def play_pause(d, k):
    poly(d, k, [(3.5, 5), (3.5, 19), (13, 12)])
    line(d, k, [(16, 6), (16, 18)], 2.4)
    line(d, k, [(20.5, 6), (20.5, 18)], 2.4)


def next_track(d, k):
    poly(d, k, [(5, 5), (5, 19), (15.5, 12)])
    line(d, k, [(18.5, 5.5), (18.5, 18.5)], 2.4)


def prev_track(d, k):
    poly(d, k, [(19, 5), (19, 19), (8.5, 12)])
    line(d, k, [(5.5, 5.5), (5.5, 18.5)], 2.4)


def power(d, k):
    arc(d, k, (4, 5, 20, 21), -50, 230)
    line(d, k, [(12, 2.5), (12, 11)])


ICONS = {
    "music": music,
    "tune": sliders,
    "reload": reload,
    "toggle": play_pause,
    "next": next_track,
    "prev": prev_track,
    "quit": power,
}

for name, draw in ICONS.items():
    for size in SIZES:
        img, d, k = canvas(size)
        draw(d, k)
        small = img.resize((size, size), Image.LANCZOS)
        (OUT / f"{name}_{size}.a8").write_bytes(small.tobytes())
        if size == 32:
            small.save(OUT.parent.parent / "target" / f"menu-preview-{name}.png")
print("ok")
