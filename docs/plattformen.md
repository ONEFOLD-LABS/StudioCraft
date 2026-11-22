# Plattformen und Auslieferung

StudioCraft ist ein Projekt von ONEFOLD LABS; Zielsysteme und Paketformate gelten für alle ONEFOLD-LABS-Projekte. Paketaufbau und Build-Skripte sind an [storytold/soundcraft](https://github.com/storytold/soundcraft) angelehnt (Apache-2.0, Release v0.20.1, Stand 2026-11-18, siehe `NOTICE`).

## Zielplattformen und Pakete

| System | Architekturen | Pakete |
| --- | --- | --- |
| Linux | x86_64, aarch64 | AppImage, deb, rpm, Flatpak, pkg.tar.zst (Arch), tar.gz |
| Linux | riscv64 | deb, rpm, tar.gz |
| Windows | x64, arm64 | MSI, portables zip |
| macOS | universal (Intel + Apple Silicon) | DMG, CLI als zip |
| FreeBSD | x86_64, aarch64 | tar.gz, pkg |
| Web | wasm32 | zip |

Jedes Release bringt `SHA256SUMS.txt` mit. Windows- und macOS-Pakete werden code-signiert (macOS auch notarisiert), sobald die Zertifikate als Secrets hinterlegt sind; bis dahin sind sie unsigniert bzw. ad hoc signiert.

## Bausteine

| Bereich | Craft-Standard | Stand StudioCraft |
| --- | --- | --- |
| Audio-I/O | cpal 0.18 (ASIO als Feature, `wasm-bindgen` fürs Web) | umgesetzt, Linux getestet (PipeWire, ALSA) |
| GUI | egui/eframe 0.37 + `craft-ui` | egui/eframe 0.37 umgesetzt (Projektfenster-Prototyp); `craft-ui` folgt, sobald Docking und Icons gebraucht werden |
| Audio-Dateien | `craft-audio` aus craft-libs | noch `hound` (nur WAV), Wechsel geplant |
| MIDI | `craft-midi` | offen |
| Plugins | CLAP überall, VST3, AU nur macOS (eigene Host-Crates) | CLAP-Inserts und -Instrumente (`clap-sys` + `libloading` wie SoundCraft), Parameter-Regler, Plugin-Zustand im Projekt, Plugin-Fenster (schwebend überall, eingebettet unter X11; eingebettet unter Windows/macOS, VST3, AU offen) |
| Fenster | winit mit X11 und Wayland unter Linux | über eframe (Features `wayland`, `x11`) |
| Plugin-Fenster | CLAP: eigenes Wayland-Fenster, sonst eingebettet über XWayland; LV2/VST3: über XWayland | `crates/plugin-host/src/gui.rs` |
| Plugin im eigenen Prozess | Gemeinsamer Speicher (Datei mit `mmap`, unter Windows File-Mapping) und zwei Verbindungen: Unix-Socket im Laufzeitordner (Linux, macOS, FreeBSD), unter Windows TCP auf der Loopback-Adresse mit geheimem Token | `crates/plugin-host/src/bridge/` |
| Übersetzungen | `craft-i18n` | offen |
| Schriften | `craft-fonts` | offen |
| Updates | `craft-update` | offen |

Hausregeln der Familie: nur Rust, kein Tauri, Electron oder handgeschriebenes JS/TS. Plattformspezifisches nur hinter `cfg` oder Features, nie als einziger Weg.

## Repository-Aufbau (an SoundCraft angeglichen)

```
apps/studiocraft        Desktop-App          (vorhanden)
apps/studiocraft-cli    Kommandozeile        (vorhanden)
apps/studiocraft-web    WASM-Build           (vorhanden)
crates/*                Bibliotheken         (vorhanden)
xtask/                  cargo xtask ci, version, layers  (geplant)
packaging/{linux,windows,macos,freebsd,web} vorhanden
.github/workflows/      ci.yml, release.yml über storytold/craft-common-actions  (geplant)
```

Releases laufen über den eigenen Workflow `.github/workflows/release.yml`: Ein Tag `v*` baut alle Pakete und legt einen Release-Entwurf an; nachts und auf Abruf läuft derselbe Bau als Probelauf ohne Veröffentlichung. Den gemeinsamen Release-Workflow aus `storytold/craft-common-actions` nutzt StudioCraft nicht, weil er dortige Branches, Umgebungen und App-Kennungen voraussetzt (Issue #31); CI-Gate und FreeBSD-Prüfung kommen weiter aus `storytold/craft-common-actions`.

## Audio-Hosts je System

| System | Hosts | Auswahl |
| --- | --- | --- |
| Linux | PipeWire (Standard), ALSA; JACK mit `--features jack` | `--audio-host <name>` |
| FreeBSD | ALSA (über alsa-plugins-oss auf OSS); PipeWire mit `--features pipewire`, JACK mit `--features jack` | ebenso |
| Windows | WASAPI (Shared Mode, Low-Latency geplant: Issue #110); ASIO nur im Selbstbau mit `--features asio`, nicht in den Paketen (Lizenz des SDK, Issue #107) | ebenso |
| macOS | CoreAudio; JACK mit `--features jack` | ebenso |
| Web | Web Audio | automatisch |

`studiocraft --list-hosts` zeigt, was im jeweiligen Build steckt.

## Abgrenzung zu SoundCraft

SoundCraft und StudioCraft haben jeweils ein eigenes Bedienkonzept. Gemeinsame Bausteine (craft-libs) werden geteilt, die Bedienung bleibt getrennt.

## Windows-Pakete

`packaging/windows/package.ps1 -Arch x64|arm64` baut mit statischer C-Laufzeit (kein VC++-Redistributable) und erzeugt das MSI (WiX v5, Installation für alle Benutzer) und ein portables zip. Das MSI legt einen Startmenü-Eintrag an, macht StudioCraft zur Standard-App für `.studiocraft` und bietet sich für WAV, FLAC, AIFF und MP3 unter „Öffnen mit“ an. `studiocraft.exe` ist ein GUI-Programm ohne Konsolenfenster (Symbol und Versionsangaben über `apps/studiocraft/build.rs`); Ausgaben auf der Kommandozeile liefert `studiocraft-cli.exe`. Signiert wird nur, wenn das Secret `WINDOWS_CERTIFICATE` gesetzt ist.

## macOS-Pakete

`packaging/macos/package.sh [--arch universal|aarch64|x86_64]` baut ein Universal-Binary (Intel und Apple Silicon, ab macOS 11), legt `StudioCraft.app` an (Info.plist mit Mikrofon-Hinweis, Symbol, gehärtete Laufzeit) und erzeugt ein DMG zum Ziehen in „Programme“ sowie die Kommandozeile als zip. Die Berechtigungen erlauben den Audio-Eingang und das Laden fremd signierter Plugins (`disable-library-validation`). Ohne Zertifikat wird ad hoc signiert; mit den Secrets `APPLE_CERTIFICATE` (Developer ID) und `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` signiert und notarisiert das Skript. Die App ist Standard für `.studiocraft`-Projekte und bietet sich für WAV, AIFF, FLAC und MP3 unter „Öffnen mit“ an. Der Finder übergibt Dokumente als Ereignis an den App-Delegate statt als Startparameter; `apps/studiocraft/src/macos_open.rs` hängt dafür `application:openURLs:` an winits Delegate und öffnet die Dateien wie Startparameter (beim Start und im laufenden Betrieb).

## Linux-Pakete

`packaging/linux/package.sh` baut tar.gz, AppImage, deb und rpm. deb und rpm installieren den Dateityp `application/x-studiocraft-project` (`*.studiocraft`), aktualisieren Desktop-, Dateityp- und Symbol-Datenbanken und empfehlen lilv für LV2-Plugins. Aus dem Tarball entstehen ohne erneutes Kompilieren:

- `flatpak-bundle.sh`: ein Flatpak-Bundle (freedesktop-Laufzeit 25.08). Rechte: Wayland/X11, GPU, PipeWire-Socket und PulseAudio, `--device=all` für MIDI über den ALSA-Sequencer, das Home-Verzeichnis (Projekte werden über Pfade geöffnet), `~/.clap`, `~/.vst3` und `~/.lv2` nur lesend. Plugins aus den Flathub-Erweiterungen `org.freedesktop.LinuxAudio.Plugins.*` werden über `CLAP_PATH`, `VST3_PATH` und `LV2_PATH` gefunden. LV2 fehlt im Flatpak vorerst, weil die Laufzeit kein lilv mitbringt.
- riscv64: `package.sh --target riscv64gc-unknown-linux-gnu` kompiliert auf x86_64 quer (tar.gz, deb, rpm; kein AppImage) und startet beide Programme zum Test über qemu-user.
- `arch-package.sh`: ein Arch-Paket (`pkg.tar.zst`, x86_64) über `makepkg`; lilv ist optional.

Der Release-Workflow installiert deb und rpm testweise unter Debian 13, Ubuntu 24.04 und Fedora und startet beide Programme, ebenso das Arch-Paket und das Flatpak.

## FreeBSD

`packaging/freebsd/package.sh` baut auf FreeBSD ein tar.gz für `/usr/local` und ein natives `pkg` (Abhängigkeiten alsa-lib und alsa-plugins). Ton läuft über ALSA, das alsa-plugins auf OSS (sound(4)) umleitet. MIDI fehlt auf FreeBSD vorerst: midir hat dort kein Backend (`crates/engine/src/midi_backend.rs` meldet das, statt den Build zu brechen). Der Workflow `.github/workflows/freebsd.yml` baut und testet über den gemeinsamen Craft-Workflow in einer FreeBSD-VM und prüft das Paket. Für Releases baut `.github/workflows/release.yml` tar.gz und `pkg` für x86_64 in einer eigenen FreeBSD-VM (nur Release-Build, ohne Tests); aarch64 wird bisher nur gebaut und getestet, nicht paketiert.

## Web

`apps/studiocraft-web` ist dasselbe Projektfenster im Browser (eframe-Web: WebGPU, sonst WebGL2). `packaging/web/package.sh` baut mit `trunk build --release` eine statische Seite und packt sie als `studiocraft-web-<version>.zip`. Ton gibt es über Web Audio; Browser erlauben ihn erst nach einer Eingabe, deshalb startet die Ausgabe beim ersten Klick oder Tastendruck im Fenster (aus Rust, ohne eigenes JavaScript). Dateien gehen über die Datei-APIs des Browsers: *Speichern* lädt das Projekt als `Name.studiocraft.zip` herunter (derselbe Projektordner wie auf der Platte, Audio als verlustfreies WAV), *Öffnen* nimmt so ein zip oder eine einzelne `.studiocraft`-Datei (dann ohne Audio), *Audio importieren* wählt Dateien aus, *Audio-Mixdown exportieren* lädt ein WAV herunter. Im Browser fehlen Plugins (die Plugin-Menüs sagen das; plugin-host stellt dort Platzhalter bereit), Aufnahme, MIDI und die automatische Sicherung; Rendern läuft ohne eigenen Thread.

Zum lokalen Bauen: rustup mit dem Ziel `wasm32-unknown-unknown` und `trunk` (`cargo install trunk --locked`).
