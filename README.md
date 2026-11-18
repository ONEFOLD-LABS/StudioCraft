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
cargo run -p studiocraft-cli -- <datei.wav> --loop 1 5  # Kommandozeile: Enter = Start/Stopp, q = Ende
```

Bedienung im Projektfenster: Tasten 1–0 wählen Werkzeuge (Auswahl, Trennen, Löschen, Stumm, Zeichnen, Zoom, Wiedergabe sind aktiv), Leertaste Start/Stopp, `/` Cycle, J Snap, H/G Zoom, Strg + Mausrad Zoom, Umschalt + Mausrad horizontal. Im Lineal: Klick setzt den Cursor, Ziehen den Cycle-Bereich, Strg/Alt + Klick den linken/rechten Locator. Events mit Strg + Ziehen kopieren, Alt hebt Snap auf. WAV-Dateien lassen sich ins Projektfenster ziehen: auf eine Audio-Spur an die Mausposition, sonst auf eine neue Spur. Mute und Solo wirken sofort auf die Wiedergabe.

Benötigt Rust 1.85 oder neuer (Edition 2024). Unter Linux zusätzlich die Entwicklerdateien von ALSA und PipeWire (`libasound`, `libpipewire-0.3`) sowie `clang`. `cargo run -- --list-hosts` zeigt die verfügbaren Audio-Hosts, `--audio-host alsa` wählt einen anderen.

## Lizenz

Wahlweise unter [MIT](LICENSE-MIT) oder [Apache 2.0](LICENSE-APACHE) lizenziert, wie die anderen Craft-Apps. Siehe [`NOTICE`](NOTICE).
