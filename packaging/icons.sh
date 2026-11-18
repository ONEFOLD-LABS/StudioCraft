#!/usr/bin/env bash
# Render every app icon file from assets/app-icon/icon.svg (48 px and up) and icon-small.svg (16-32 px).
#
# Needs resvg or rsvg-convert, and Python 3 with Pillow for the .ico and .icns.
# Run from anywhere; redraw the SVGs first with packaging/make_icon_svg.py if the mark changed.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dir="$root/assets/app-icon"
big="$dir/icon.svg"
small="$dir/icon-small.svg"

render() { # svg size out
    if command -v resvg >/dev/null; then resvg -w "$2" -h "$2" "$1" "$3"
    else rsvg-convert -w "$2" -h "$2" "$1" -o "$3"; fi
}

for size in 16 24 32 48 64 128 256 512; do
    src="$big"; [ "$size" -le 32 ] && src="$small"
    out="$dir/hicolor/${size}x${size}/apps"
    mkdir -p "$out"
    render "$src" "$size" "$out/studiocraft.png"
done
mkdir -p "$dir/hicolor/scalable/apps"
cp "$big" "$dir/hicolor/scalable/apps/studiocraft.svg"
render "$big" 1024 "$dir/studiocraft-1024.png"

python3 - "$dir" <<'PY'
import sys
from pathlib import Path
from PIL import Image

d = Path(sys.argv[1])
png = lambda s: Image.open(d / f"hicolor/{s}x{s}/apps/studiocraft.png").convert("RGBA")
big = Image.open(d / "studiocraft-1024.png").convert("RGBA")

# Windows: every size drawn for itself, the small ones from icon-small.svg
sizes = [16, 24, 32, 48, 64, 128, 256]
png(256).save(d / "studiocraft.ico", sizes=[(s, s) for s in sizes], append_images=[png(s) for s in sizes[:-1]])

# macOS: the mark on Apple's icon grid, an 824/1024 body with a transparent margin
def mac(n):
    body = round(n * 824 / 1024)
    im = Image.new("RGBA", (n, n))
    im.alpha_composite(big.resize((body, body), Image.LANCZOS), ((n - body) // 2, (n - body) // 2))
    return im
mac(512).save(d / "studiocraft-macos-512.png")
mac(1024).save(d / "studiocraft.icns", append_images=[mac(n) for n in (16, 32, 64, 128, 256, 512)])
PY

echo "app icons written to $dir"
