"""Screenshots and GIFs for the README, from the notch preview.

Needs the dev server (`npx vite`, port 1420), Chrome or Edge, and Pillow
(`pip install pillow`). Run from the repo root:

    python scripts/readme_shots.py

Writes docs/images/*.png and *.gif. The preview uses fictional data only.
"""

import os
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageChops

BASE = "http://localhost:1420/dev/notch-preview.html"
OUT = Path("docs/images")
BROWSERS = [
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
]
W, H, SCALE = 1100, 760, 2


def browser() -> str:
    for b in BROWSERS:
        if os.path.exists(b):
            return b
    sys.exit("Chrome/Edge não encontrado")


def raw(query: str, budget_ms: int) -> Image.Image:
    tmp = Path(tempfile.mkdtemp()) / "s.png"
    profile = tempfile.mkdtemp()
    subprocess.run(
        [
            browser(),
            "--headless=new",
            "--disable-gpu",
            "--hide-scrollbars",
            "--mute-audio",
            f"--user-data-dir={profile}",
            f"--window-size={W},{H}",
            f"--force-device-scale-factor={SCALE}",
            f"--virtual-time-budget={budget_ms}",
            f"--screenshot={tmp}",
            f"{BASE}?shot=1&{query}",
        ],
        check=True,
        capture_output=True,
        timeout=90,
    )
    return Image.open(tmp).convert("RGB")


def content_box(img: Image.Image, pad: int) -> tuple[int, int, int, int]:
    bg = Image.new("RGB", img.size, img.getpixel((4, img.height - 4)))
    diff = ImageChops.difference(img, bg).convert("L").point(lambda v: 255 if v > 12 else 0)
    l, t, r, b = diff.getbbox() or (0, 0, img.width, img.height)
    return max(0, l - pad), max(0, t - pad), min(img.width, r + pad), min(img.height, b + pad)


def shot(name: str, query: str, budget: int = 3500, pad: int = 28, box=None) -> Image.Image:
    img = raw(query, budget)
    img = img.crop(box or content_box(img, pad))
    img.save(OUT / f"{name}.png", optimize=True)
    print("ok", name, img.size)
    return img


def gif(name: str, frames: list[Image.Image], ms: int, width: int) -> None:
    box = None
    for f in frames:  # same size for every frame
        box = f.size if box is None else (max(box[0], f.size[0]), max(box[1], f.size[1]))
    canvas = []
    for f in frames:
        c = Image.new("RGB", box, f.getpixel((2, f.height - 2)))
        c.paste(f, ((box[0] - f.width) // 2, 0))
        h = round(c.height * width / c.width)
        canvas.append(c.resize((width, h), Image.LANCZOS).convert("P", palette=Image.ADAPTIVE, colors=128))
    canvas[0].save(OUT / f"{name}.gif", save_all=True, append_images=canvas[1:], duration=ms, loop=0, optimize=True)
    print("ok", name, "gif", len(frames), "frames")


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)

    # The compact notch.
    shot("notch-pill", "widgets=1", 4200)
    shot("notch-game", "widgets=fps&game=1", 6200)
    shot("notch-offline", "widgets=1&net=off", 4200)
    shot("notch-alert", "alert=1", 2600)
    shot("notch-youtube", "mediaapp=1&spotify=paused", 2200)

    # Live widgets (rings and FPS moving).
    frames = [raw("widgets=fps", 2600 + i * 1000) for i in range(6)]
    box = content_box(frames[-1], 24)
    gif("notch-widgets", [f.crop(box) for f in frames], 700, 900)

    # Tabs of the open panel, all cropped to the same box.
    tabs = [
        ("panel-home", "open=home"),
        ("panel-agenda", "open=agenda"),
        ("panel-spotify", "open=music"),
        ("panel-mail", "open=mail"),
        ("panel-whatsapp", "open=whatsapp"),
        ("panel-whatsapp-chat", "open=whatsapp&wachat=Ana Souza&call=active&theme=glass"),
        ("panel-files", "open=files&fq=contrato"),
        ("panel-notes", "open=notes"),
        ("panel-pc", "open=pc&scroll=end"),
        ("panel-routines", "open=tasks&sub=Rotinas"),
        ("panel-contacts", "open=tasks&sub=Contatos"),
        ("panel-season-natal", "open=home&season=natal&theme=glass"),
        ("panel-season-junina", "open=home&season=junina&theme=glass"),
        ("panel-liquid", "open=home&theme=liquid&glint=420,230"),
        ("settings-seasons", "open=settings&sub=Notch&season=auto&scroll=Temas da época"),
        ("settings-obs-backup", "open=settings&scroll=OBS Studio"),
        ("settings-ai", "open=settings&scroll=Trocar de IA automaticamente"),
        ("settings-notch", "open=settings&sub=Notch&scroll=Informações no notch"),
        ("settings-voice", "open=settings&enrolled=1&scroll=Reconhecimento da sua voz"),
        ("settings-visual", "open=settings&sub=Notch&theme=glass"),
        ("settings-voice-picker", "open=settings&scroll=Voz do Jarvis"),
        ("panel-glass", "open=home&widgets=1&theme=glass"),
        ("settings-folders", "open=settings&scroll=Editor de código dos projetos"),
    ]
    imgs = {n: raw(q, 3800) for n, q in tabs}
    box = content_box(imgs["panel-home"], 24)
    for n, img in imgs.items():
        img.crop(box).save(OUT / f"{n}.png", optimize=True)
        print("ok", n)
    # The three looks of the pill, one under the other.
    pills = []
    for theme in ["graphite", "glass", "midnight"]:
        p = raw(f"widgets=1&theme={theme}", 4200)
        pills.append(p.crop(content_box(p, 40)))
    w = max(i.width for i in pills)
    sheet = Image.new("RGB", (w, sum(i.height for i in pills) + 40), pills[0].getpixel((2, 2)))
    y = 0
    for i in pills:
        sheet.paste(i, ((w - i.width) // 2, y))
        y += i.height + 20
    sheet.save(OUT / "notch-themes.png", optimize=True)
    print("ok notch-themes")

    tour = ["panel-home", "panel-agenda", "panel-spotify", "panel-mail", "panel-files", "panel-notes", "panel-whatsapp", "panel-pc"]
    gif("tour", [imgs[n].crop(box) for n in tour], 1600, 900)


if __name__ == "__main__":
    main()
