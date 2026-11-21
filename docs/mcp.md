# MCP-Server für KI-Agenten

`studiocraft-cli mcp` stellt StudioCraft über das [Model Context Protocol](https://modelcontextprotocol.io)
bereit: JSON-RPC 2.0, eine Nachricht je Zeile über stdin/stdout (Protokoll `2025-06-18`, ältere
Clients mit `2025-03-26` und `2024-11-05` werden angenommen). So kann ein Agent (Claude Code,
Claude Desktop, Codex, Cursor …) Projekte anlegen, bearbeiten, speichern und als Mixdown rendern.

Der Server steckt in `crates/automation` (`studiocraft-automation`). Das Protokoll kommt aus
`craft-mcp-server`, der Steuerkanal zur App aus `craft-control` (beide craft-libs), wie in den
anderen Craft-Apps. Es gibt zwei Betriebsarten:

| Betriebsart | Aufruf | Was gesteuert wird |
|---|---|---|
| **Kopflos** (Standard) | `studiocraft-cli mcp …` | Ein eigenes `ProjectWindow` ohne Fenster und ohne Audiogerät |
| **Connect** | `studiocraft --control 7850` und `studiocraft-cli mcp … --connect 7850` | Das offene Fenster der laufenden App; zusätzlich `transport`, `ui_screenshot`, `ui_inspect` und `ui_click` |

In beiden Fällen laufen die Werkzeuge über denselben Code (`exec.rs`) auf einem
`ProjectWindow`. Undo-Verlauf, Import, Speichern und Mixdown verhalten sich also wie in der App.

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

## Die laufende App steuern (`--connect`)

```sh
studiocraft --control 7850 &
studiocraft-cli mcp --automation-read-root /arbeit/projekt --automation-write-root /arbeit/projekt \
                    --connect 7850
```

- `studiocraft --control <port>` öffnet den Steuerkanal auf `127.0.0.1:<port>`, nie im Netz.
  Jeder Start erzeugt ein neues Token (32 Zufallsbytes) und legt es in
  `<Einstellungsordner>/control/<port>.token` ab, lesbar nur für den eigenen Benutzer. Ohne
  dieses Token nimmt die App keine Anfrage an. `current.port` daneben nennt den zuletzt
  geöffneten Port. Beim Beenden entfernt die App beide Dateien wieder.
  `STUDIOCRAFT_CONTROL_DIR` legt einen anderen Ordner fest, `STUDIOCRAFT_CONTROL_TOKEN` ein festes
  Token (für Testumgebungen).
- `--connect` ohne Port nimmt den zuletzt geöffneten Steuerkanal, sonst Port 7850. Angegeben
  werden darf ein Port oder `127.0.0.1:<port>`; andere Rechner werden abgelehnt. Der Server
  verbindet sich erst beim ersten Aufruf, die App darf also später starten. Läuft sie nicht,
  meldet das Werkzeug das als Fehler.
- Die Ordnergrenzen gelten auch hier. Der MCP-Server prüft jeden Pfad, bevor er die Anfrage
  schickt, und schickt ihn absolut. Speichert `save_project` ohne `path`, gilt das für den
  Speicherort, den das offene Projekt in der App hat.
- Die App führt jede Anfrage zwischen zwei Bildern aus. Die Person am Rechner sieht jede Änderung
  sofort und kann sie mit Strg+Z zurücknehmen. `new_project` und `open_project` verwerfen
  ungesicherte Änderungen nur mit `discard_changes: true`.
- Eine Bearbeitung wird nie zweimal geschickt: Reißt die Verbindung mitten in einer Anfrage ab,
  meldet das Werkzeug einen Fehler, und der Agent prüft mit `inspect_project`, ob sie angekommen
  ist.
- `ui_inspect` liest den Baum, den egui für Bildschirmleser (AccessKit) baut. Die App baut ihn nur
  für die Anfrage und schaltet ihn danach wieder ab, außer ein Bildschirmleser braucht ihn.
  Beschriftungen folgen der Sprache der App. Ids gelten, solange das Element da ist.
- `ui_click` klickt wie die Maus in der Mitte des Elements, also nur, was im Fenster sichtbar
  ist. Ein Dialog, der durch den Klick aufgeht, ist beim nächsten `ui_inspect` zu sehen.
- Die App selbst prüft keine Pfade. Wer die Token-Datei lesen kann (also derselbe Benutzer), kann
  sie direkt steuern, wie bei den anderen Craft-Apps. Die Begrenzung gilt für den MCP-Server.

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
| `transport` | Nur mit `--connect`: Play, Stopp, Cursor setzen |
| `ui_screenshot` | Nur mit `--connect`: PNG des App-Fensters, wie es gerade aussieht |
| `ui_inspect` | Nur mit `--connect`: Bedienelemente des Fensters, wie ein Bildschirmleser sie sieht (Id, Rolle, Beschriftung, Wert, Rechteck in Punkten, Zustand); Filter nach Text und Rolle |
| `ui_click` | Nur mit `--connect`: Mausklick auf ein Element (Id oder exakte Beschriftung) oder einen Punkt; linke, rechte oder mittlere Taste, Doppelklick, gehaltene Tasten |

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

- Kopflos werden Plugins nicht geladen und nicht gerendert, und es gibt keine Wiedergabe,
  keine Aufnahme und kein MIDI-Gerät. Mit `--connect` spielt die App selbst ab.
- Der Mixdown rendert in beiden Betriebsarten ohne Plugins. MIDI-Parts bleiben darin stumm; das
  Ergebnis von `export_mixdown` sagt das dazu.
- Tastendrücke, Texteingabe, Ziehen und Scrollen gibt es noch nicht, ebenso wenig Aufnahme über
  den Agenten. Klicks erreichen keine Plugin-Fenster (eigene Fenster der Plugins).
- Mixer-Routing, Sends, Inserts, Automation und Spuren löschen fehlen noch als Werkzeuge.
