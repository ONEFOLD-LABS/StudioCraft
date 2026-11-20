# StudioCraft

StudioCraft is a cross-platform digital audio workstation written in Rust. It is an independent
personal project. Its operating concept follows the established DAW workflow described in
[`docs/spec.md`](docs/spec.md) and is built clean-room: no code, assets or texts from other
products, no decompiling or reverse engineering.

The project follows the conventions of the open Craft app family (pure Rust, egui, permissive
licences, same packaging layout) and uses some of its shared libraries (`craft-audio`,
`craft-window`) and CI actions. It is not an official part of that family.

> **Status:** early development, no release yet. Linux is the platform used day to day; Windows
> and macOS builds are produced in CI but are not tested by hand. The user interface is German for
> now; language switching is planned ([#99](https://github.com/xXCrusaderXx/StudioCraft/issues/99)).

## Features

- Project window with inspector, track list, event display and lower zone (mixer, key editor,
  sample editor); tools on keys 1–0
- Audio, instrument, MIDI, group, FX, folder, marker and tempo tracks
- Recording of audio and MIDI with cycle takes, punch in/out, pre-count, take lanes and comping
- Key editor, quantize, crossfade and fade editors
- Mixer with channel strips (input stage, pre filters, EQ, inserts before and after the fader,
  sends), solo modes, peak hold, saved mixer views, input and output busses
- Automation (read/write), tempo track and time signatures, plugin delay compensation
- CLAP and LV2 plugins (instruments and effects), plugin state saved with the project; the plugin
  scan runs in child processes and blocks plugins that crash
- Render in place, track freeze, stem export, audio mixdown (WAV, FLAC, AIFF)
- Project folder with open JSON format, autosave and crash recovery
- Dark and light theme

The full target behaviour is in [`docs/spec.md`](docs/spec.md); platforms and packages are in
[`docs/plattformen.md`](docs/plattformen.md) (both in German).

## Building

Requires Rust 1.85 or newer (edition 2024). On Linux you also need the development files of ALSA
and PipeWire (`libasound`, `libpipewire-0.3`) and `clang`. LV2 support needs `lilv` installed at
run time; without it, LV2 plugins are simply not offered.

```sh
cargo build --release
cargo run --release -p studiocraft                          # project window with demo tracks
cargo run --release -p studiocraft -- a.wav b.wav           # each file on its own audio track
cargo run --release -p studiocraft -- Song/Song.studiocraft # open a project
cargo run --release -p studiocraft -- --list-hosts          # audio hosts in this build
cargo run -p studiocraft-cli -- file.wav --loop 1 5         # command-line player
cargo run -p studiocraft-cli -- mcp --automation-read-root DIR --automation-write-root DIR
                                                            # MCP server for AI agents (docs/mcp.md)
cargo run --release -p studiocraft -- --control 7850       # app with a control channel for --connect
```

## Layout

| Path | Role |
| --- | --- |
| `apps/studiocraft` | Desktop app (eframe/egui) |
| `apps/studiocraft-cli` | Command-line player for tests and scripting; `mcp` serves the MCP server |
| `crates/project` | Project model, edits, tempo map, versioned file format |
| `crates/engine` | Real-time audio: transport, mixing and routing, recording, MIDI, export |
| `crates/plugin-host` | Plugin hosting (CLAP, LV2) |
| `crates/graph` | Signal-graph helpers |
| `crates/ui` | Project window, mixer, editors, menus, theme, settings |
| `crates/automation` | MCP server for AI agents: headless, or driving the running app (`--connect`) |
| `xtask` | `cargo xtask ci`, `cargo xtask version` |
| `packaging/` | Package scripts per platform |

## Checks and packages

```sh
cargo xtask ci                   # rustfmt, clippy without warnings, all tests (same as CI)
cargo xtask version [set 0.2.0]  # show or set the version
packaging/linux/package.sh       # dist/release/: AppImage, tar.gz, deb, rpm
```

A tag `v*` builds packages for Linux, Windows and macOS and creates a draft release. Installers
for Windows and macOS, Flatpak, FreeBSD and a web build are in progress (milestone 0.5).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). If you work with AI agents, also read
[AGENTS.md](AGENTS.md). Changes reach `main` only through pull requests.

## Licence

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option. Third-party
credits are in [`NOTICE`](NOTICE).
