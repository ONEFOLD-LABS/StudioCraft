# StudioCraft

Plattformübergreifende DAW in Rust für Linux, Windows, macOS, FreeBSD und das Web. Teil der Craft-Familie. Das Bedienkonzept orientiert sich an klassischen DAWs und wird im Cleanroom-Verfahren nachgebaut.

- **Plattformen und Pakete:** [`docs/plattformen.md`](docs/plattformen.md)
- **Spezifikation:** [`docs/spec.md`](docs/spec.md) (lebende Fassung: https://claude.ai/artifact/JPZqqo7GT4DvJsx8RHg6kL)
- **Cleanroom-Regel:** Implementiert wird nur gegen die Spec. Kein Dekompilieren, kein Reverse Engineering von fremder Programme oder fremden Projektdateien, keine fremde Marken oder -Grafiken.

## Aufbau

| Crate | Aufgabe |
| --- | --- |
| `apps/studiocraft` | Startprogramm |
| `crates/engine` | Transport, Echtzeit-Kern, Audio-Ausgabe über cpal |
| `crates/graph` | Audiograph: Kanäle, Inserts, Sends, Busse |
| `crates/plugin-host` | CLAP, später VST3 und LV2 |
| `crates/project` | Projektmodell, Positionen, Tempo-Map |
| `crates/ui` | Projektfenster, Mixer, Editoren |

## Bauen

```sh
cargo build
cargo test
cargo run -- <datei.wav>                    # interaktiv: Enter = Start/Stopp, c = Cycle, q = Ende
cargo run -- <datei.wav> --loop 1 5         # Takt 1 bis 5 als Cycle
```

Benötigt Rust 1.85 oder neuer (Edition 2024). Unter Linux zusätzlich die Entwicklerdateien von ALSA und PipeWire (`libasound`, `libpipewire-0.3`) sowie `clang`. `cargo run -- --list-hosts` zeigt die verfügbaren Audio-Hosts, `--audio-host alsa` wählt einen anderen.

## Lizenz

Wahlweise unter [MIT](LICENSE-MIT) oder [Apache 2.0](LICENSE-APACHE) lizenziert, wie die anderen Craft-Apps. Siehe [`NOTICE`](NOTICE).
