# StudioCraft-App-Icon

**Welle:** eine Audio-Wellenform aus isometrischen Blöcken, die um eine Mittellinie schwingt. Jeder
Block hat drei sichtbare Flächen (oben, links, rechts), jede mit einem Zwei-Farb-Verlauf aus der
Weinrot-Rampe; die Fugen zwischen den Flächen sind offen (transparent). Kein Hintergrund und keine
Kachel, das Zeichen steht frei.

Das Prinzip (isometrischer Körper, Flächenverläufe, offene Fugen) folgt SolidCraft. Die Form ist
eigenständig gezeichnet; SolidCraft-Marke und -Icon stehen unter deren Brand-Lizenz und wurden weder
nachgezeichnet noch verwendet.

## Farben

| Fläche | Von | Nach |
|---|---|---|
| Oben | `#d9587a` | `#b8324f` |
| Links | `#b52e50` | `#8e1f3c` |
| Rechts | `#8a1f3a` | `#5c1126` |

App-Farbe (Akzent im UI, siehe `crates/ui/src/theme.rs`): Weinrot `#9e2343`.

## Geometrie

512 × 512 viewBox, transparenter Hintergrund. `icon.svg` hat fünf Blöcke und 11 px Fugen und gilt ab
48 px; `icon-small.svg` hat drei Blöcke und 30 px Fugen, damit das Zeichen bei 16–32 px nicht
zuläuft. Die macOS-Dateien setzen das Zeichen auf Apples Raster (824/1024 Körper, Rand transparent).

## Herkunft und Lizenz

Von Grund auf als berechnete SVG-Pfade erzeugt (`packaging/make_icon_svg.py`), keine Vorlage
nachgezeichnet. MIT OR Apache-2.0 wie der Code.

## Dateien

| Datei | Wofür |
|---|---|
| `icon.svg` | Das Zeichen als Vektor, ab 48 px |
| `icon-small.svg` | Vereinfachte Fassung für 16–32 px |
| `studiocraft-1024.png` | 1024-px-Render |
| `studiocraft-macos-512.png` | Fenster-/Dock-Icon zur Laufzeit unter macOS (Apple-Raster) |
| `studiocraft.icns` | macOS-Bundle-Icon |
| `studiocraft.ico` | Windows-Icon (16–256 px, kleine Stufen aus `icon-small.svg`) |
| `hicolor/<n>x<n>/apps/studiocraft.png` | Linux/FreeBSD-Icon-Theme 16–512 px, dazu `scalable/` |

## Neu erzeugen

```sh
uv run --with shapely python packaging/make_icon_svg.py   # nur wenn sich das Zeichen ändert
packaging/icons.sh                                         # alle Größen, .ico, .icns
```

`icons.sh` braucht `resvg` oder `rsvg-convert` und Python 3 mit Pillow.
