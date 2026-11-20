"""Draw StudioCraft's app icon, the "Ausklang" mark, as plain SVG paths.

A disc in the wine ramp with a decaying wave cut out of it: a struck tone that rings out, quick
rise, then slowly fading swings. The wave is a real hole in the disc (computed geometry, no SVG
mask), so every icon renderer shows the background through it.

Two drawings come out of it, both 512 x 512 with a transparent background:

  assets/app-icon/icon.svg        2.5 swings, thinner line, for 48 px and up
  assets/app-icon/icon-small.svg  fewer swings and a thicker line, for 16 to 32 px

Needs Python 3 with shapely (for example `uv run --with shapely python packaging/make_icon_svg.py`).
Run from the repository root; `packaging/icons.sh` renders all sizes from the two SVGs.
"""
import math
from pathlib import Path

from shapely.geometry import LineString, Point

SIZE = 512
CENTER = SIZE / 2
RADIUS = 226

# Wine ramp around the app colour #9e2343: light at the top left, dark at the bottom right.
LIGHT, DARK = "#d9587a", "#8e1f3c"

AUSKLANG = {"amp": 150, "swings": 2.5, "line": 30, "x0": 118, "x1": 400}
AUSKLANG_SMALL = {"amp": 140, "swings": 1.8, "line": 44, "x0": 118, "x1": 400}


def wave(spec, steps=240):
    """Points of the decaying wave: quick rise over the first 12 %, then an exponential fade."""
    pts = []
    for i in range(steps + 1):
        t = i / steps
        x = spec["x0"] + (spec["x1"] - spec["x0"]) * t
        envelope = min(1.0, t / 0.12) * math.exp(-1.9 * t)
        y = CENTER - spec["amp"] * envelope * math.sin(2 * math.pi * spec["swings"] * t)
        pts.append((x, y))
    return pts


def draw(spec, title):
    disc = Point(CENTER, CENTER).buffer(RADIUS, quad_segs=64)
    cut = LineString(wave(spec)).buffer(spec["line"] / 2, quad_segs=16)
    shape = disc.difference(cut)

    d = ""
    for poly in [shape] if shape.geom_type == "Polygon" else list(shape.geoms):
        for ring in [poly.exterior, *poly.interiors]:
            d += "M" + " L".join(f"{x:.1f} {y:.1f}" for x, y in list(ring.coords)[:-1]) + " Z"

    defs = (f'<linearGradient id="face" x1="0.2" y1="0" x2="0.8" y2="1">'
            f'<stop offset="0" stop-color="{LIGHT}"/><stop offset="1" stop-color="{DARK}"/>'
            f"</linearGradient>")
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SIZE} {SIZE}" width="{SIZE}" '
            f'height="{SIZE}" role="img"><title>{title}</title><defs>{defs}</defs>'
            f'<path fill-rule="evenodd" fill="url(#face)" d="{d}"/></svg>\n')


if __name__ == "__main__":
    out = Path("assets/app-icon")
    out.mkdir(parents=True, exist_ok=True)
    (out / "icon.svg").write_text(draw(AUSKLANG, "StudioCraft"))
    (out / "icon-small.svg").write_text(draw(AUSKLANG_SMALL, "StudioCraft"))
    print("wrote", out / "icon.svg", "and", out / "icon-small.svg")
