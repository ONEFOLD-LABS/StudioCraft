# StudioCraft

Plattformübergreifende DAW in Rust für Linux, Windows, macOS, FreeBSD und das Web. Teil der Craft-Familie. Das Bedienkonzept orientiert sich an klassischen DAWs und wird im Cleanroom-Verfahren nachgebaut.

- **Plattformen und Pakete:** [`docs/plattformen.md`](docs/plattformen.md)
- **Spezifikation:** [`docs/spec.md`](docs/spec.md)
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

Bedienung im Projektfenster: Tasten 1–0 wählen Werkzeuge (alle außer Zeitkorrektur sind aktiv: Bereichsauswahl mit Entf zum Herausschneiden, Kleben verbindet mit dem nächsten Event, Alt+Klick mit allen folgenden; P setzt die Locatoren auf die Auswahl), Leertaste Start/Stopp, `/` Cycle, J Snap, H/G Zoom, Strg + Mausrad Zoom, Umschalt + Mausrad horizontal. Im Lineal: Klick setzt den Cursor, Ziehen den Cycle-Bereich, Strg/Alt + Klick den linken/rechten Locator. Events mit Strg + Ziehen kopieren, Alt hebt Snap auf. Audiodateien (WAV, AIFF, FLAC, MP3, Ogg, Opus, M4A u. a. über craft-audio) lassen sich ins Projektfenster ziehen: auf eine Audio-Spur an die Mausposition, sonst auf eine neue Spur. Mute und Solo wirken sofort auf die Wiedergabe.

Mit dem Auswahl-Werkzeug hat jedes Event Smart-Bereiche: am Rand in der unteren Hälfte die Länge ändern, in der oberen Hälfte Fade-In/Fade-Out ziehen (auch an den Griffen des ausgewählten Events). Strg+Z macht rückgängig, Strg+Umschalt+Z (oder Strg+Y) stellt wieder her.

**Projekte:** Strg+N neu, Strg+O öffnen, Strg+S speichern, Strg+Umschalt+S speichern unter, Strg+Alt+S neue Version (`Song-01`, `Song-02` …). Ein Projekt ist ein Ordner mit `<Name>.studiocraft` (offenes JSON) und `Audio/`; importierte Dateien werden beim Speichern dorthin kopiert. Vor dem Verwerfen ungesicherter Änderungen fragt StudioCraft nach. *Datei → Audio-Mixdown exportieren* schreibt den Bereich zwischen den Locatoren als WAV, FLAC oder AIFF (24 Bit, je nach Dateiendung).

**Aufnahme:** R im Spurkopf macht eine Audio-Spur aufnahmebereit, im Inspector wählt man den Eingang (Stereopaar oder Mono-Kanal des Interfaces). ⏺ in der Transportleiste oder `*` startet und stoppt. Jede Spur bekommt eine eigene WAV-Datei in `Audio/` (vor dem ersten Speichern im temporären Ordner). Mit Cycle wird jeder Durchgang ein eigener Take, der jüngste liegt oben; Punch-In (I) und Punch-Out (O) beschränken die Aufnahme auf die Locatoren. Bei überlappenden Events klingt wie in klassischen DAWs nur das oberste. Aufnahmen werden automatisch um die vom Treiber gemeldete Ein- und Ausgabelatenz verschoben, sodass sie zum Gehörten passen.

**Plugins (CLAP):** Im Inspector unter *Inserts* lassen sich bis zu acht Effekte je Audio-Spur einfügen, sortiert nach Hersteller. Gesucht wird in `CLAP_PATH` und den Standardorten (`~/.clap`, `/usr/lib/clap`, `/usr/local/lib/clap`; unter Windows und macOS die dortigen CLAP-Ordner). Jeder Insert lässt sich umgehen, entfernen und über Regler je Parameter einstellen; alles wird im Projekt gespeichert und ist rückgängig zu machen. *Fenster* öffnet das eigene Fenster des Plugins (schwebend, sonst unter Linux eingebettet in ein X11-Fenster, unter Wayland über XWayland). Der komplette Plugin-Zustand (Presets, interne Einstellungen) wird mit dem Projekt gespeichert.

**Instrument-Spuren:** *Projekt → Instrument-Spur hinzufügen* legt eine Spur mit einem CLAP-Instrument an (Strg+T legt eine Audio-Spur an). Parts (mit dem Zeichnen-Werkzeug) spielen ihre Noten über das Instrument; Instrumente und Effekte laufen auch im Stopp weiter, damit sie ausklingen. Parameter großer Plugins lassen sich im Inspector durchsuchen.

**Key-Editor:** Doppelklick auf einen Part öffnet ihn in der unteren Zone (Reiter *Mixer* / *Key-Editor*). Klaviatur links (Klick spielt an), Notenraster mit eigenem Zoom, Velocity-Spur unten. Die Werkzeuge gelten auch hier: Auswahl verschiebt (Strg kopiert, rechter Rand ändert die Länge), Zeichnen setzt Noten und zieht sie auf, Löschen entfernt; Entf löscht ausgewählte Noten, alles rückgängig zu machen.

**MIDI-Keyboard:** Alle MIDI-Eingänge werden beim Start geöffnet. Gespielt wird auf der aufnahmebereiten Instrument-Spur, sonst auf der ausgewählten; die Noten gehen direkt in den Audio-Thread. Mit R auf einer Instrument-Spur nimmt ⏺ bzw. `*` MIDI auf (auch zusammen mit Audio): daraus wird ein Part auf Taktgrenzen, im Cycle über den Cycle-Bereich mit allen Durchgängen, Punch wirkt wie bei Audio. Controller, Pitchbend und Aftertouch gehen live ans Instrument; das Haltepedal wird bei der Aufnahme in die Notenlängen übernommen (eigene Controller-Spuren folgen).

**Tempo und Metronom:** In der Transportleiste lassen sich Tempo und Taktart einstellen (rückgängig zu machen). Wie in klassischen DAWs behalten Audio-Events ihre Zeitposition, MIDI-Parts ihre Taktposition. *Klick* bzw. C schaltet das Metronom (Takt-Eins betont).

**Gruppen, FX-Kanäle, Sends:** *Projekt → Gruppen-Spur / FX-Kanal hinzufügen*. Im Inspector legt *Ausgang* fest, ob eine Spur auf die Summe oder eine Gruppe geht; unter *Sends* gehen bis zu acht Sends (Pre/Post, ein/aus, Pegel) an FX-Kanäle oder Gruppen. Schleifen werden gar nicht erst angeboten. Gruppen und FX-Kanäle haben eigene Inserts und bleiben bei Solo hörbar.

**Erscheinungsbild:** *Ansicht → Erscheinungsbild* wählt Dunkel, Hell oder „Wie das System“; die Wahl wird gemerkt. Akzentfarbe ist das Weinrot des App-Icons, Aufnahme bleibt davon unterscheidbar orangerot.

F3 blendet den Mixer in der unteren Zone ein und aus: je Kanal Panorama, Mute/Solo, Fader mit Pegelanzeige, rechts der Ausgang. Strg + Klick setzt Fader auf 0 dB bzw. Panorama auf Mitte, Umschalt + Ziehen stellt fein ein.

Benötigt Rust 1.85 oder neuer (Edition 2024). Unter Linux zusätzlich die Entwicklerdateien von ALSA und PipeWire (`libasound`, `libpipewire-0.3`) sowie `clang`. `cargo run -- --list-hosts` zeigt die verfügbaren Audio-Hosts, `--audio-host alsa` wählt einen anderen.

## Prüfen und Pakete bauen

```sh
cargo xtask ci                      # Formatierung, Clippy ohne Warnungen, alle Tests (wie die CI)
cargo xtask version [set 0.2.0]     # Version zeigen bzw. setzen
packaging/linux/package.sh          # dist/release/: AppImage, tar.gz (deb/rpm mit nfpm)
packaging/linux/install-user.sh PFAD # Startmenü-Eintrag und Icons für AppImage/cargo-Build (nur eigener Benutzer)
```

Die CI läuft über den gemeinsamen Gate-Workflow der Craft-Familie (`storytold/craft-common-actions`). Ein Tag `v*` baut Linux-Pakete (x86_64, aarch64) und Archive für Windows und macOS als Release-Entwurf. Installer für Windows/macOS sowie FreeBSD und Web folgen.

## Mitmachen

Siehe [CONTRIBUTING.md](CONTRIBUTING.md). Wer mit KI-Agenten arbeitet, liest zusätzlich [AGENTS.md](AGENTS.md). Änderungen an `main` nur per Pull Request.

## Lizenz

Wahlweise unter [MIT](LICENSE-MIT) oder [Apache 2.0](LICENSE-APACHE) lizenziert, wie die anderen Craft-Apps. Siehe [`NOTICE`](NOTICE).
