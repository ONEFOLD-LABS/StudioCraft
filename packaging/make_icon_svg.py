"""Draw StudioCraft's app icon, the "Welle" mark, as plain SVG paths.

A waveform built from isometric blocks: each block's top, left and right face gets a two-stop
gradient in the wine ramp, and the joints between faces are open (transparent). Visibility is
computed, not faked with strokes: every face loses what lies in front of it, then each visible piece
is inset by half a joint, so only edges you can see get a joint.

Two drawings come out of it, both 512 x 512 with a transparent background:

  assets/app-icon/icon.svg        five blocks, for 48 px and up
  assets/app-icon/icon-small.svg  three blocks with wider joints, for 16 to 32 px

Needs Python 3 with shapely (for example `uv run --with shapely python packaging/make_icon_svg.py`).
Run from the repository root; `packaging/icons.sh` renders all sizes from the two SVGs.
"""
import math
from pathlib import Path

from shapely.geometry import Polygon
from shapely.ops import unary_union

SIZE = 512
COS30 = math.cos(math.radians(30))

# Wine ramp around the app colour #9e2343: top lightest, left middle, right darkest.
RAMP = {
    "top": ("#d9587a", "#b8324f"),
    "left": ("#b52e50", "#8e1f3c"),
    "right": ("#8a1f3a", "#5c1126"),
}

# (height, x position) per block; blocks are centred on z = 0 like a waveform.
WELLE = {"heights": [1.2, 2.6, 3.8, 2.2, 1.0], "step": 1.1, "width": 0.8, "depth": 1.0, "gap": 11, "margin": 26}
WELLE_SMALL = {"heights": [1.8, 3.6, 2.2], "step": 1.45, "width": 1.05, "depth": 1.05, "gap": 30, "margin": 8}


def project(x, y, z):
    return ((x - y) * COS30, (x + y) * 0.5 - z)


def block_faces(x, z, dx, dy, dz):
    """Visible faces of a block at (x, 0, z), back to front: right and left sides, then the top."""
    corners = {
        "top": [(x, 0, z + dz), (x + dx, 0, z + dz), (x + dx, dy, z + dz), (x, dy, z + dz)],
        "right": [(x + dx, 0, z), (x + dx, dy, z), (x + dx, dy, z + dz), (x + dx, 0, z + dz)],
        "left": [(x, dy, z), (x + dx, dy, z), (x + dx, dy, z + dz), (x, dy, z + dz)],
    }
    return [(shade, [project(*p) for p in corners[shade]]) for shade in ("right", "left", "top")]


def draw(spec, title):
    faces = []  # back to front
    for i, h in enumerate(spec["heights"]):
        faces += block_faces(i * spec["step"], -h / 2, spec["width"], spec["depth"], h)

    pts = [p for _, poly in faces for p in poly]
    minx, maxx = min(p[0] for p in pts), max(p[0] for p in pts)
    miny, maxy = min(p[1] for p in pts), max(p[1] for p in pts)
    scale = (SIZE - 2 * spec["margin"]) / max(maxx - minx, maxy - miny)
    ox = (SIZE - (maxx - minx) * scale) / 2 - minx * scale
    oy = (SIZE - (maxy - miny) * scale) / 2 - miny * scale

    pieces, front = [], None
    for shade, poly in reversed(faces):
        shape = Polygon([(x * scale + ox, y * scale + oy) for x, y in poly])
        visible = shape if front is None else shape.difference(front)
        front = shape if front is None else unary_union([front, shape])
        visible = visible.buffer(-spec["gap"] / 2, join_style="mitre", mitre_limit=4)
        if not visible.is_empty:
            pieces.append((shade, visible))
    pieces.reverse()

    def path(geom):
        d = ""
        for p in [geom] if geom.geom_type == "Polygon" else list(geom.geoms):
            for ring in [p.exterior, *p.interiors]:
                d += "M" + " L".join(f"{x:.1f} {y:.1f}" for x, y in list(ring.coords)[:-1]) + " Z"
        return d

    defs = "".join(
        f'<linearGradient id="{shade}" x1="0.2" y1="0" x2="0.5" y2="1">'
        f'<stop offset="0" stop-color="{a}"/><stop offset="1" stop-color="{b}"/></linearGradient>'
        for shade, (a, b) in RAMP.items())
    body = "".join(f'<path fill-rule="evenodd" fill="url(#{shade})" d="{path(g)}"/>' for shade, g in pieces)
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SIZE} {SIZE}" width="{SIZE}" height="{SIZE}" '
            f'role="img"><title>{title}</title><defs>{defs}</defs>{body}</svg>\n')


if __name__ == "__main__":
    out = Path("assets/app-icon")
    out.mkdir(parents=True, exist_ok=True)
    (out / "icon.svg").write_text(draw(WELLE, "StudioCraft"))
    (out / "icon-small.svg").write_text(draw(WELLE_SMALL, "StudioCraft"))
    print("wrote", out / "icon.svg", "and", out / "icon-small.svg")
