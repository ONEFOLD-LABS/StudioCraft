# MCP-Server für KI-Agenten

`studiocraft-cli mcp` stellt StudioCraft über das [Model Context Protocol](https://modelcontextprotocol.io)
bereit: JSON-RPC 2.0, eine Nachricht je Zeile über stdin/stdout (Protokoll `2025-06-18`, ältere
Clients mit `2025-03-26` und `2024-11-05` werden angenommen). So kann ein Agent (Claude Code,
Claude Desktop, Codex, Cursor …) Projekte anlegen, bearbeiten, speichern und als Mixdown rendern.

Der Server steckt in `crates/automation` (`studiocraft-automation`). Das Protokoll kommt aus
`craft-mcp-server` (craft-libs), wie in den anderen Craft-Apps. Die Werkzeuge steuern ein
`ProjectWindow` ohne Fenster und ohne Audiogerät. Damit laufen Undo-Verlauf, Import, Speichern
und Mixdown über denselben Code wie in der App.

## Start und Einbindung

```sh
cargo build --release -p studiocraft-cli
studiocraft-cli mcp --automation-read-root ~/Musik/Samples --automation-read-root ~/Musik/Projekte \
                    --automation-write-root ~/Musik/Projekte
```

Claude Code:

```sh
claude mcp add studiocraft -- /pfad/zu/studiocraft-cli mcp \
  --automation-read-root /arbeit/projekt --automation-write-root /arbeit/projekt
```

Andere Clients (z. B. `claude_desktop_config.json`, `.mcp.json`) erwarten dieselbe Form:

```json
{"mcpServers": {"studiocraft": {"command": "/pfad/zu/studiocraft-cli",
  "args": ["mcp", "--automation-read-root", "/arbeit/projekt", "--automation-write-root", "/arbeit/projekt"]}}}
```

Fehler und Hinweise gehen nach stderr, auf stdout stehen nur Protokollnachrichten.

## Dateizugriff begrenzen

Nach dem Craft-Standard `app-agents` (Regel 18) startet der Server nur mit beiden Schaltern:

- `--automation-read-root <ordner>`: Ordner, aus dem gelesen werden darf (`open_project`,
  `import_audio`, `import_midi`).
- `--automation-write-root <ordner>`: Ordner, in den geschrieben werden darf (`save_project` samt
  `Audio/`-Ordner und Zwischendatei daneben, `export_mixdown`, `export_midi`).

Beide Schalter dürfen mehrfach vorkommen (`--schalter ordner` oder `--schalter=ordner`). Lesen und
Schreiben sind getrennte Rechte: Soll ein Ordner beides erlauben, wird er bei beiden Schaltern
angegeben. Die Ordner müssen existieren. Ein unbekannter Schalter ist ein Fehler, damit ein
Tippfehler nie zu einem unbegrenzten Server führt.

Ein Pfad wird gegen das Arbeitsverzeichnis absolut gemacht, Links werden aufgelöst. Bei einer
Datei, die es noch nicht gibt, wird der nächste vorhandene Ordner aufgelöst, der Rest muss aus
einfachen Namen bestehen: `..` unter einem fehlenden Ordner und Links ins Leere werden abgelehnt.
Verglichen wird nach ganzen Pfadteilen (`/arbeit2/x` liegt nicht in `/arbeit`), unter Windows ohne
Groß-/Kleinschreibung. Ein Projekt, dessen Audiodateien außerhalb der Lese-Ordner liegen, wird
nicht geöffnet. Eine abgelehnte Datei ist ein normaler Werkzeugfehler, den der Agent lesen kann,
z. B. `automation path rejected: outside the write root /arbeit/projekt: /etc/x.wav`.

Nicht abgedeckt: Die eigenen Einstellungen von StudioCraft werden beim Start gelesen (nie
geschrieben). Zwischen Prüfung und Öffnen kann ein anderes Programm einen Link austauschen. Die
Ordner halten also den Agenten bei seinen Dateien, sie schützen nicht vor anderer Software.

## Werkzeuge

| Werkzeug | Zweck |
|---|---|
| `inspect_project` | Ganzes Projekt: Tempo, Taktart, Locatoren, Marker, Spuren mit Events (Ids, Positionen, Längen), Undo-Tiefe, Pfad, ungesichert |
| `get_notes` | Noten eines MIDI-Parts |
| `new_project`, `open_project`, `save_project` | Projektdateien (`.studiocraft`, Audio im Ordner `Audio/`). Speichern überschreibt eine andere vorhandene Datei nur mit `replace: true` |
| `import_audio`, `import_midi` | Audiodatei auf neue oder vorhandene Spur; Standard-MIDI-Datei als MIDI-Spuren |
| `export_mixdown` | Locator-Bereich als 24-Bit-WAV, FLAC oder AIFF; meldet die Spitze in dBFS |
| `export_midi` | MIDI- und Instrument-Spuren als MIDI-Datei |
| `add_track`, `set_track` | Spur anlegen; Name, Fader, Pan, Mute, Solo |
| `create_part`, `add_notes` | Leeren MIDI-Part anlegen; Noten hinzufügen oder ersetzen |
| `move_event`, `split_event`, `delete_events` | Events verschieben oder kopieren, teilen, löschen |
| `set_tempo`, `add_marker`, `set_locators` | Tempo und Taktart, Positions- und Cycle-Marker, Locatoren |
| `undo`, `redo` | Verlauf |

Ressource: `studiocraft://project` (wie `inspect_project`).

Konventionen:

- 960 Ticks je Viertel. Positionen sind Ticks oder Text wie im Lineal, ab 1 gezählt (`5.1.1.0` =
  Takt 5). Längen sind Ticks oder Text ab 0 gezählt (`1.0.0.0` = ein Takt). Noten liegen relativ
  zum Part-Anfang.
- Spuren über Index (ab 0) oder exakten Namen, Events über ihre Id.
- Jede Bearbeitung ist ein Undo-Schritt. Auf die Platte kommt erst etwas mit `save_project` oder
  einem Export.
- Fehler kommen als Werkzeugergebnis mit `isError: true` zurück, nie als Absturz.

## Grenzen (Stand dieser Version)

- Nur der kopflose Betrieb: Der Server steuert nicht das laufende Fenster der App. Ein
  Steuerkanal zur App (wie `--connect` bei anderen Craft-Apps, mit Screenshots) folgt getrennt.
- Plugins werden nicht geladen und nicht gerendert. MIDI-Parts klingen im Mixdown nur, wenn ein
  Instrument sie spielt, also in der App. Das Ergebnis von `export_mixdown` sagt das dazu.
- Keine Wiedergabe, keine Aufnahme, kein MIDI-Gerät.
- Mixer-Routing, Sends, Inserts, Automation und Spuren löschen fehlen noch als Werkzeuge.
