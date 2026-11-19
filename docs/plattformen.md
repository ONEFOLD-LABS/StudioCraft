# Plattformen und Auslieferung

StudioCraft läuft wie die übrigen Craft-Apps auf allen Systemen und wird in denselben Formaten ausgeliefert. Vorbild ist [storytold/soundcraft](https://github.com/storytold/soundcraft) (Release v0.20.1, Stand 2026-11-18). Dessen Konventionen gelten für die ganze Familie.

## Zielplattformen und Pakete

| System | Architekturen | Pakete |
| --- | --- | --- |
| Linux | x86_64, aarch64 | AppImage, deb, rpm, Flatpak, pkg.tar.zst (Arch), tar.gz |
| Linux | riscv64 | deb, rpm, tar.gz |
| Windows | x64, arm64 | MSI, portables zip |
| macOS | universal (Intel + Apple Silicon) | DMG, CLI als zip |
| FreeBSD | x86_64, aarch64 | tar.gz |
| Web | wasm32 | zip |

Jedes Release bringt `SHA256SUMS.txt` und signierte Pakete mit (`.asc`/`.sig`). Windows und macOS werden code-signiert und notarisiert.

## Bausteine

| Bereich | Craft-Standard | Stand StudioCraft |
| --- | --- | --- |
| Audio-I/O | cpal 0.18 (ASIO als Feature, `wasm-bindgen` fürs Web) | umgesetzt, Linux getestet (PipeWire, ALSA) |
| GUI | egui/eframe 0.37 + `craft-ui` | egui/eframe 0.37 umgesetzt (Projektfenster-Prototyp); `craft-ui` folgt, sobald Docking und Icons gebraucht werden |
| Audio-Dateien | `craft-audio` aus craft-libs | noch `hound` (nur WAV), Wechsel geplant |
| MIDI | `craft-midi` | offen |
| Plugins | CLAP überall, VST3, AU nur macOS (eigene Host-Crates) | CLAP-Inserts und -Instrumente (`clap-sys` + `libloading` wie SoundCraft), Parameter-Regler, Plugin-Zustand im Projekt, Plugin-Fenster (schwebend überall, eingebettet unter X11; eingebettet unter Windows/macOS, VST3, AU offen) |
| Fenster | winit mit X11 und Wayland unter Linux | über eframe (Features `wayland`, `x11`) |
| Übersetzungen | `craft-i18n` | offen |
| Schriften | `craft-fonts` | offen |
| Updates | `craft-update` | offen |

Hausregeln der Familie: nur Rust, kein Tauri, Electron oder handgeschriebenes JS/TS. Plattformspezifisches nur hinter `cfg` oder Features, nie als einziger Weg.

## Repository-Aufbau (an SoundCraft angeglichen)

```
apps/studiocraft        Desktop-App          (vorhanden)
apps/studiocraft-cli    Kommandozeile        (vorhanden)
apps/studiocraft-web    WASM-Build           (geplant)
crates/*                Bibliotheken         (vorhanden)
xtask/                  cargo xtask ci, version, layers  (geplant)
packaging/{linux,windows} vorhanden; {macos,freebsd,web} geplant
.github/workflows/      ci.yml, release.yml über storytold/craft-common-actions  (geplant)
```

Releases laufen über den gemeinsamen Workflow `storytold/craft-common-actions/.github/workflows/release.yml` mit `app-name: studiocraft`. Er erwartet die `packaging/`-Skripte in der Struktur oben.

## Audio-Hosts je System

| System | Hosts | Auswahl |
| --- | --- | --- |
| Linux, FreeBSD | PipeWire (Standard), ALSA; JACK mit `--features jack` | `--audio-host <name>` |
| Windows | WASAPI (Shared Mode, Low-Latency geplant: Issue #110); ASIO nur im Selbstbau mit `--features asio`, nicht in den Paketen (Lizenz des SDK, Issue #107) | ebenso |
| macOS | CoreAudio; JACK mit `--features jack` | ebenso |
| Web | Web Audio | automatisch |

`studiocraft --list-hosts` zeigt, was im jeweiligen Build steckt.

## Abgrenzung zu SoundCraft

SoundCraft und StudioCraft haben jeweils ein eigenes Bedienkonzept. Gemeinsame Bausteine (craft-libs) werden geteilt, die Bedienung bleibt getrennt.

## Windows-Pakete

`packaging/windows/package.ps1 -Arch x64|arm64` baut mit statischer C-Laufzeit (kein VC++-Redistributable) und erzeugt das MSI (WiX v5, Installation für alle Benutzer) und ein portables zip. Das MSI legt einen Startmenü-Eintrag an, macht StudioCraft zur Standard-App für `.studiocraft` und bietet sich für WAV, FLAC, AIFF und MP3 unter „Öffnen mit“ an. `studiocraft.exe` ist ein GUI-Programm ohne Konsolenfenster (Symbol und Versionsangaben über `apps/studiocraft/build.rs`); Ausgaben auf der Kommandozeile liefert `studiocraft-cli.exe`. Signiert wird nur, wenn das Secret `WINDOWS_CERTIFICATE` gesetzt ist.
