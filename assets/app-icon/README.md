# StudioCraft-App-Icon

**Ausklang:** eine Scheibe im Weinrot-Verlauf, aus der eine ausklingende Welle ausgespart ist –
ein angeschlagener Ton, der kurz ansteigt und dann in immer kleineren Schwingungen verklingt. Die
Welle ist ein echtes Loch in der Scheibe (berechnete Geometrie, keine SVG-Maske), der Hintergrund
scheint durch. Kein weiterer Hintergrund und keine Kachel.

Dasselbe Zeichen zeichnet die App in ihrer eigenen Titelleiste (Vektor, `crates/ui/src/icons.rs`,
`app_mark`).

## Farben

| Verlauf | Von (oben links) | Nach (unten rechts) |
|---|---|---|
| Scheibe | `#d9587a` | `#8e1f3c` |

App-Farbe (Akzent im UI, siehe `crates/ui/src/theme.rs`): Weinrot `#9e2343`.

## Geometrie

512 × 512 viewBox, transparenter Hintergrund, Scheibe mit Radius 226. `icon.svg` hat 2,5
Schwingungen und eine 30 px breite Aussparung und gilt ab 48 px; `icon-small.svg` hat 1,8
Schwingungen und 44 px, damit die Welle bei 16–32 px offen bleibt. Die macOS-Dateien setzen das
Zeichen auf Apples Raster (824/1024 Körper, Rand transparent).

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
