# KI-Assistent

Ansicht → KI-Assistent öffnet am rechten Fensterrand ein Chatfenster. Dort beschreibt man in
eigenen Worten, was im Projekt passieren soll, zum Beispiel „Lege eine Bass-Spur mit einem Part
über 4 Takte an und stelle das Tempo auf 100“. Ein Sprachmodell führt das mit denselben
Werkzeugen aus wie der MCP-Server ([mcp.md](mcp.md)).

- Jede Änderung ist ein eigener Undo-Schritt. Strg+Z nimmt sie wie gewohnt zurück.
- Unter der Nachricht steht jedes ausgeführte Werkzeug mit seinen Argumenten. Ist ein Aufruf
  fehlgeschlagen, ist die Zeile rot.
- „Neu“ beginnt ein neues Gespräch, „Abbrechen“ verwirft eine laufende Anfrage.
- Je Nachricht sind höchstens 24 Runden von Werkzeugaufrufen erlaubt. Danach hält der Assistent
  an und wartet auf die nächste Nachricht.

## Modell einrichten

Unter „Verbindung“:

| Feld | Bedeutung |
|---|---|
| Adresse | Basisadresse einer OpenAI-kompatiblen API (`…/v1`). Voreingestellt ist Ollama: `http://localhost:11434/v1`. LM Studio: `http://localhost:1234/v1`. |
| Modell | Name des Modells, z. B. `qwen2.5:7b`. „Modelle abfragen“ listet, was der Server anbietet. |
| API-Schlüssel | Nur für Dienste, die einen verlangen; lokale Server brauchen keinen. |

Das Modell muss Werkzeugaufrufe (tool calling) beherrschen. Kleine Modelle verwechseln
Positionen und Längen leichter.

- **Adresse und Modell** stehen in `assistant.json` im Einstellungsordner.
- **Der Schlüssel** wird nur mit „Schlüssel speichern“ gespeichert, dann in `assistant.key`.
  Unter Linux, macOS und FreeBSD ist die Datei nur für den eigenen Benutzer lesbar (Modus 0600).
  Ohne Haken gilt der Schlüssel nur bis zum Beenden. Alternativ setzt man ihn über die
  Umgebungsvariable `STUDIOCRAFT_AI_KEY`, dann wird er gar nicht gespeichert.

## Datenschutz

- **Lokal** (Adresse auf `localhost`, `127.0.0.1` oder `::1`): Nichts verlässt den Rechner.
- **Dienst im Netz:** Vor der ersten Anfrage an einen Host fragt die App einmal nach. Ab dann
  gehen die Nachrichten und die Projektdaten, die die Werkzeuge liefern (Spurnamen, Events,
  Noten, Tempo), an diesen Host. Die Zustimmung gilt je Host und steht in `assistant.json`;
  wer sie zurücknehmen will, löscht dort den Host aus `consented`.
- **Verschlüsselung:** Zu anderen Rechnern geht nur `https://`. Geprüft wird mit den
  Zertifikaten des Betriebssystems. `http://` ist nur zum eigenen Rechner erlaubt.

## Was der Assistent darf

- Er hat dieselben Werkzeuge wie der MCP-Server: Spuren, Parts, Noten, Events, Tempo, Marker,
  Locatoren, Undo/Redo, Transport, Import, Export und Speichern.
- `new_project` und `open_project` fehlen. Das offene Projekt kann er also nicht ersetzen.
- Fensterwerkzeuge (`ui_*`) hat er auch nicht.
- Dateien liest und schreibt er nur im Ordner des Projekts, und erst wenn das Projekt
  gespeichert ist.

## Grenzen (Stand dieser Version)

- Antworten erscheinen erst vollständig, nicht Wort für Wort.
- Markdown in Antworten wird als Text gezeigt.
- Keine Bilder oder Screenshots an das Modell.
- Plugins und Mixer-Routing stehen dem Assistenten noch nicht als Werkzeuge zur Verfügung.

## Testen gegen ein echtes Modell

```sh
STUDIOCRAFT_AI_TEST_MODEL=qwen2.5:7b cargo test -p studiocraft-assistant -- --ignored --nocapture
```

`STUDIOCRAFT_AI_TEST_URL` wählt einen anderen Server.
