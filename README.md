# StudioCraft

Plattformübergreifende DAW in Rust für Linux, Windows, macOS, FreeBSD und das Web. Teil der Craft-Familie. Das Bedienkonzept orientiert sich an klassischen DAWs und wird im Cleanroom-Verfahren nachgebaut.

- **Plattformen und Pakete:** [`docs/plattformen.md`](docs/plattformen.md)
- **Spezifikation:** [`docs/spec.md`](docs/spec.md) (lebende Fassung: https://claude.ai/artifact/JPZqqo7GT4DvJsx8RHg6kL)
- **Cleanroom-Regel:** Implementiert wird nur gegen die Spec. Kein Dekompilieren, kein Reverse Engineering von fremder Programme oder fremden Projektdateien, keine fremde Marken oder -Grafiken.

## Aufbau

| Crate | Aufgabe |
| --- | --- |
| `apps/studiocraft` | Desktop-App (egui) |
| `apps/studiocraft-cli` | Kommandozeile |
| `crates/engine` | Transport, Echtzeit-Kern, Audio-Ausgabe über cpal |
| `crates/graph` | Audiograph: Kanäle, Inserts, Sends, Busse |
| `crates/plugin-host` | CLAP, später VST3 und LV2 |
| `crates/project` | Projektmodell, Positionen, Tempo-Map |
| `crates/ui` | Projektfenster, Mixer, Editoren (egui) |

## Bauen

```sh
cargo build
cargo test
cargo run --release -p studiocraft                      # Projektfenster mit 100 Demo-Spuren
cargo run --release -p studiocraft -- a.wav b.wav       # jede Datei auf eigener Audio-Spur
cargo run --release -p studiocraft -- Song/Song.studiocraft  # Projekt öffnen
cargo run -p studiocraft-cli -- <datei.wav> --loop 1 5  # Kommandozeile: Enter = Start/Stopp, q = Ende
```

Bedienung im Projektfenster: Tasten 1–0 wählen Werkzeuge (Auswahl, Trennen, Löschen, Stumm, Zeichnen, Zoom, Wiedergabe sind aktiv), Leertaste Start/Stopp, `/` Cycle, J Snap, H/G Zoom, Strg + Mausrad Zoom, Umschalt + Mausrad horizontal. Im Lineal: Klick setzt den Cursor, Ziehen den Cycle-Bereich, Strg/Alt + Klick den linken/rechten Locator. Events mit Strg + Ziehen kopieren, Alt hebt Snap auf. WAV-Dateien lassen sich ins Projektfenster ziehen: auf eine Audio-Spur an die Mausposition, sonst auf eine neue Spur. Mute und Solo wirken sofort auf die Wiedergabe.

Mit dem Auswahl-Werkzeug hat jedes Event Smart-Bereiche: am Rand in der unteren Hälfte die Länge ändern, in der oberen Hälfte Fade-In/Fade-Out ziehen (auch an den Griffen des ausgewählten Events). Strg+Z macht rückgängig, Strg+Umschalt+Z (oder Strg+Y) stellt wieder her.

**Projekte:** Strg+N neu, Strg+O öffnen, Strg+S speichern, Strg+Umschalt+S speichern unter, Strg+Alt+S neue Version (`Song-01`, `Song-02` …). Ein Projekt ist ein Ordner mit `<Name>.studiocraft` (offenes JSON) und `Audio/`; importierte Dateien werden beim Speichern dorthin kopiert. Vor dem Verwerfen ungesicherter Änderungen fragt StudioCraft nach. *Datei → Audio-Mixdown exportieren* schreibt den Bereich zwischen den Locatoren als 24-Bit-WAV.

**Aufnahme:** R im Spurkopf macht eine Audio-Spur aufnahmebereit, im Inspector wählt man den Eingang (Stereopaar oder Mono-Kanal des Interfaces). ⏺ in der Transportleiste oder `*` startet und stoppt. Jede Spur bekommt eine eigene WAV-Datei in `Audio/` (vor dem ersten Speichern im temporären Ordner). Mit Cycle wird jeder Durchgang ein eigener Take, der jüngste liegt oben; Punch-In (I) und Punch-Out (O) beschränken die Aufnahme auf die Locatoren. Bei überlappenden Events klingt wie in klassischen DAWs nur das oberste. Aufnahmen werden automatisch um die vom Treiber gemeldete Ein- und Ausgabelatenz verschoben, sodass sie zum Gehörten passen.

**Plugins (CLAP):** Im Inspector unter *Inserts* lassen sich bis zu acht Effekte je Audio-Spur einfügen, sortiert nach Hersteller. Gesucht wird in `CLAP_PATH` und den Standardorten (`~/.clap`, `/usr/lib/clap`, `/usr/local/lib/clap`; unter Windows und macOS die dortigen CLAP-Ordner). Jeder Insert lässt sich umgehen, entfernen und über Regler je Parameter einstellen; alles wird im Projekt gespeichert und ist rückgängig zu machen. Eigene Plugin-Fenster folgen.

**Instrument-Spuren:** *Projekt → Instrument-Spur hinzufügen* legt eine Spur mit einem CLAP-Instrument an (Strg+T legt eine Audio-Spur an). Parts (mit dem Zeichnen-Werkzeug) spielen ihre Noten über das Instrument; Instrumente und Effekte laufen auch im Stopp weiter, damit sie ausklingen. Parameter großer Plugins lassen sich im Inspector durchsuchen.

F3 blendet den Mixer in der unteren Zone ein und aus: je Kanal Panorama, Mute/Solo, Fader mit Pegelanzeige, rechts der Ausgang. Strg + Klick setzt Fader auf 0 dB bzw. Panorama auf Mitte, Umschalt + Ziehen stellt fein ein.

Benötigt Rust 1.85 oder neuer (Edition 2024). Unter Linux zusätzlich die Entwicklerdateien von ALSA und PipeWire (`libasound`, `libpipewire-0.3`) sowie `clang`. `cargo run -- --list-hosts` zeigt die verfügbaren Audio-Hosts, `--audio-host alsa` wählt einen anderen.

## Lizenz

Wahlweise unter [MIT](LICENSE-MIT) oder [Apache 2.0](LICENSE-APACHE) lizenziert, wie die anderen Craft-Apps. Siehe [`NOTICE`](NOTICE).
