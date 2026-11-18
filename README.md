# StudioCraft

Native Linux-DAW in Rust. Teil der Craft-Familie. Das Bedienkonzept orientiert sich an klassischen DAWs und wird im Cleanroom-Verfahren nachgebaut.

- **Spezifikation:** [`docs/spec.md`](docs/spec.md) (lebende Fassung: https://claude.ai/artifact/JPZqqo7GT4DvJsx8RHg6kL)
- **Cleanroom-Regel:** Implementiert wird nur gegen die Spec. Kein Dekompilieren, kein Reverse Engineering von fremder Programme oder fremden Projektdateien, keine fremde Marken oder -Grafiken.

## Aufbau

| Crate | Aufgabe |
| --- | --- |
| `app` (`studiocraft`) | Startprogramm |
| `crates/engine` | Transport, Audio-I/O, Echtzeit-Thread |
| `crates/graph` | Audiograph: Kanäle, Inserts, Sends, Busse |
| `crates/plugin-host` | CLAP, später VST3 und LV2 |
| `crates/project` | Projektmodell, Positionen, Tempo-Map |
| `crates/ui` | Projektfenster, Mixer, Editoren |

## Bauen

```sh
cargo build
cargo test
cargo run
```

Benötigt Rust 1.85 oder neuer (Edition 2024).

## Lizenz

Wahlweise unter [MIT](LICENSE-MIT) oder [Apache 2.0](LICENSE-APACHE) lizenziert, wie die anderen Craft-Apps. Siehe [`NOTICE`](NOTICE).
