# StudioCraft – guide for contributors working with AI agents

This file is for AI coding agents (Claude Code, Codex, Copilot, Cursor, …) and for the people who
run them. `CLAUDE.md` points here. Read it fully before the first change; [CONTRIBUTING.md](CONTRIBUTING.md)
applies to agents as well.

StudioCraft is a clean-room, pure-Rust digital audio workstation with the established
operating concept of classic DAWs, as described in `docs/spec.md`, without code, assets or texts
from any other product. It is an independent project, not part of the Craft family of open apps,
but follows its conventions ([storytold/craft-contrib](https://github.com/storytold/craft-contrib)
house rules).

## Rules that are never bent

1. **A person is accountable.** Whoever runs an agent answers for every commit, pull request and
   review comment it produces. Read the diff before you open a pull request.
2. **`main` changes only through a pull request** with green CI. Never push to `main`, never
   force-push a shared branch. Branch protection enforces this.
3. **Clean room.** Learn from observed behaviour (running the product, its public manual and
   videos) and public specifications. Never decompile, disassemble or copy anything from other
   DAWs: no code, icons, images, sounds, presets, templates, wording beyond generic feature
   names. Do not name other DAWs or their vendors anywhere in the project (UI, code, comments,
   docs, commits, issues, pull requests). Do not copy GPL/LGPL/AGPL code either; permissive
   sources (MIT, Apache-2.0, BSD) only, credited in `NOTICE`.
4. **Licences.** Dependencies and assets must be MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib,
   OFL-1.1, CC0-1.0 or CC-BY-4.0. Anything else: stop and ask a maintainer. Every bundled asset is
   original or openly licensed and credited (see `assets/app-icon/README.md`, `NOTICE`).
5. **Report only what ran.** State which commands you ran and what you could not test (audio
   hardware, MIDI devices, plugin windows, other operating systems). Never claim a test passed
   that you did not run.
6. **No secrets or personal data** in code, commits, logs, test fixtures or requests to external
   services. Commit with an address you are happy to publish (e.g. your GitHub noreply address).
7. **Stop and ask** before: adding a licence outside the list, publishing or releasing, adding a
   remote, touching secrets, spending money, destructive git (force push, history rewrite,
   deleting branches others use), deleting user data. Otherwise keep working.

## Before you change code

- Read the relevant part of [docs/spec.md](docs/spec.md) (the behaviour we aim for) and the code
  you touch. Open a definition before using it; do not guess APIs.
- Check open issues and pull requests for the same work. Comment on the issue you take.
- Keep a change focused: one feature or fix per pull request, with tests. Do not reformat or
  refactor unrelated code. CI workflow changes go in their own pull request.

## Gate: run before every commit

```sh
cargo xtask ci     # cargo fmt --check, clippy -D warnings, all tests (exactly what CI runs)
```

UI changes: also render screenshots and look at them.

```sh
STUDIOCRAFT_SHOTS=/some/dir cargo test -p studiocraft-ui   # writes PNGs of the render tests
```

Plugin editor windows need a display: `STUDIOCRAFT_GUI_TEST=1 cargo test -p studiocraft-plugin-host --test gui_main_thread`.

## Architecture in one screen

| Crate | Role |
|---|---|
| `crates/project` | Project model (tracks, events, notes, routing), edits, tempo map, versioned JSON file format. Pure logic, heavily tested. |
| `crates/engine` | Real-time audio: cpal output/input, transport, playlist (`Song`), mixing and routing, recording, latency clock, MIDI (midir), metronome, export. |
| `crates/plugin-host` | CLAP hosting (clap-sys + libloading): scan, instances, parameters, state, editor windows. The only crate with substantial `unsafe`. |
| `crates/graph` | Signal-graph helpers. |
| `crates/ui` | egui UI: project window, timeline, inspector, mixer, key editor, menus, theme, settings. |
| `apps/studiocraft` | The eframe desktop app. |
| `apps/studiocraft-cli` | Command-line player for tests and scripting. |
| `xtask` | `cargo xtask ci`, `cargo xtask version`. |
| `packaging/` | Package scripts per platform (Linux today; Windows, macOS, FreeBSD, web follow). |

## Real-time audio rules (engine and plugin processing)

The audio callback must never block or allocate.

- No allocation, no locks, no I/O, no logging, no `Arc` drops of large data on the audio thread.
- Main thread → audio thread through the lock-free command queue (`rtrb`); status and meters
  back through atomics. Replaced songs and plugin chains travel back on the garbage queue and are
  freed on the main thread (`EngineControl::collect_garbage`).
- Buffers are allocated up front (e.g. bus buffers, `MAX_BUSES`).
- If the command queue is full, `try_send` hands the command back; never drop a plugin chain while
  the UI still holds handles to it.

## CLAP plugin rules

- Respect CLAP thread rules: `init`, `activate`, GUI, state and parameter info on the main thread;
  `process` on the audio thread only.
- Provide buffers for every port, including side chains.
- A `PluginHandle` (main-thread access) must be dropped before its instance.
- Every `unsafe` block gets a `// SAFETY:` comment explaining why it is sound. Keep `unsafe` inside
  `crates/plugin-host` (and the few documented spots elsewhere).

## Robustness

A crash loses takes, so this outranks features.

- New non-test code does not `unwrap()`, `expect()` or `panic!`; return `Result` and show the
  error in the UI. Existing `unwrap` calls outside tests are debt: remove them when you touch the
  code, never add new ones.
- Treat project files, audio files, MIDI input and plugin output as hostile: bounds-checked
  access, checked arithmetic, capped allocations, NaN guarded.
- Every crash fix comes with a test that failed before.

## UI and design language

- Follow the operating concept described in `docs/spec.md`: project window with
  inspector, track list, event display, lower zone (mixer, key editor), toolbar tools on keys 1–0,
  transport bar, the same shortcuts where the spec lists them.
- Colours come from `crates/ui/src/theme.rs` tokens (`theme::tokens(ctx)`); do not hard-code
  `Color32` values except user track colours. Check dark and light theme.
- Draw only what is visible (virtualised rows and strips); keep a frame well under 2 ms for
  1000 tracks.
- Every interactive widget has an accessible label; render tests run `check_accessibility`.
- UI text is written in German in the code and doubles as the translation key: wrap every
  user-visible text in `crate::i18n::tr("…")` (or `trf("… {name} …", &[("name", &v)])` for
  values) and add a `German<TAB>English` line to `crates/ui/i18n/en.tsv`; a test fails otherwise.
  Text used as a key or stored in files stays untranslated; translate it only where it is shown.
  Keep wording short and consistent with existing menus.

## Platforms

StudioCraft targets Linux (x86_64, aarch64, riscv64), Windows, macOS (universal), FreeBSD and the
web (wasm), like every Craft app. Do not introduce platform-only dependencies without a `cfg`
fallback; check `docs/plattformen.md`. No Tauri, Electron, webviews or handwritten JS/TS.

## Commits and pull requests

- Commit messages in **English**, imperative subject (≤ 72 chars), body explaining what and why.
- One logical change per commit; the tree passes `cargo xtask ci` at every commit.
- Branch names: `feat/…`, `fix/…`, `docs/…`, `ci/…`.
- Fill in the pull request template: what changed, why, how it was tested, what could not be tested.
