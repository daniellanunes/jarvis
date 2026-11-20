"""App, installer and tray icons from docs/brand/jarvis-mark.svg (the same
mark as the website). Needs Chrome/Edge and Pillow.

    python scripts/app_icons.py
"""

import os
import subprocess
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
MARK = ROOT / "docs/brand/jarvis-mark.svg"
OUT = ROOT / "src-tauri/icons"
BROWSERS = [
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
]


def render(size: int) -> Image.Image:
    svg = MARK.read_text(encoding="utf-8").replace('width="512" height="512"', f'width="{size}" height="{size}"')
    d = Path(tempfile.mkdtemp())
    (d / "m.html").write_text(f"<html><body style='margin:0;background:transparent'>{svg}</body></html>", encoding="utf-8")
    out = d / "m.png"
    chrome = next(b for b in BROWSERS if os.path.exists(b))
    subprocess.run(
        [chrome, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--default-background-color=00000000",
         f"--user-data-dir={tempfile.mkdtemp()}", f"--window-size={size},{size}", "--force-device-scale-factor=1",
         f"--screenshot={out}", (d / "m.html").as_uri()],
        check=True, capture_output=True, timeout=60,
    )
    return Image.open(out).convert("RGBA").crop((0, 0, size, size))


def main() -> None:
    big = render(1024)
    for name, s in {"32x32.png": 32, "128x128.png": 128, "128x128@2x.png": 256, "icon.png": 512}.items():
        big.resize((s, s), Image.LANCZOS).save(OUT / name, optimize=True)
    big.resize((256, 256), Image.LANCZOS).save(OUT / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    print("icons ok")


if __name__ == "__main__":
    main()
