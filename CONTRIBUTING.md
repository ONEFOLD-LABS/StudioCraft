# Contributing to StudioCraft

Thanks for helping. StudioCraft is a clean-room Rust DAW with the operating concept described in
`docs/spec.md`.
Contributions of code, tests, documentation, translations, bug reports and observations of how
things should behave are all welcome.

Working with an AI agent? Read [AGENTS.md](AGENTS.md) too; its rules apply to everyone.

## Ground rules

- **Clean room.** Never copy code, icons, images, sounds, presets or text from other DAWs,
  proprietary or GPL-licensed, and do not name other DAWs or their vendors anywhere in the
  project. Describing observed behaviour ("Alt-click with the
  split tool splits the event at every bar") is fine and helpful; copying resources is not.
- **Licence.** By contributing you agree that your work is licensed under MIT OR Apache-2.0, like
  the rest of the project. New dependencies and assets must be permissively licensed (see AGENTS.md).
- **`main` is protected.** Every change goes through a pull request and needs green CI.
- **Be kind.** Assume good intent, review the code, not the person.

## Getting started

Requirements: Rust (stable, edition 2024), and on Linux the development packages for ALSA,
PipeWire, X11/Wayland and fontconfig:

```sh
# Debian/Ubuntu
sudo apt install pkg-config libasound2-dev libpipewire-0.3-dev libclang-dev \
  libxkbcommon-dev libwayland-dev libx11-dev libxcb1-dev libxcursor-dev libxrandr-dev libxi-dev \
  libfontconfig1-dev libfreetype-dev libgl1-mesa-dev
# Arch
sudo pacman -S alsa-lib pipewire clang libxkbcommon wayland libx11 fontconfig
```

```sh
cargo run --release -p studiocraft            # the app
cargo run --release -p studiocraft-cli -- --help
cargo xtask ci                                # fmt check, clippy -D warnings, all tests
```

## Workflow

1. Look for an issue or open one. For larger features, describe the planned behaviour first
   (ideally with a reference to `docs/spec.md`) and wait for a maintainer's go-ahead.
2. Fork and create a branch: `feat/…`, `fix/…`, `docs/…`, `ci/…`.
3. Make focused commits with English messages (imperative subject, body with the why).
4. Run `cargo xtask ci`. For UI changes, render screenshots
   (`STUDIOCRAFT_SHOTS=dir cargo test -p studiocraft-ui`) and attach before/after images to the PR.
5. Open a pull request and fill in the template. Mention what you could not test (hardware,
   operating systems, plugins).
6. Address review feedback with new commits; the maintainer squashes or merges.

## Code standards (short version)

- No allocations, locks or I/O on the audio thread.
- No new `unwrap()`/`expect()`/`panic!` outside tests; return errors.
- `unsafe` only in `crates/plugin-host`, each block with a `// SAFETY:` comment.
- Colours from the theme tokens, accessible labels on every widget.
- Tests for every fix and feature; the project model (`crates/project`) is pure logic and should
  be tested thoroughly.

## Reporting bugs

Use the bug report template. Include the version (`studiocraft --version`), operating system,
audio backend and device, steps to reproduce and, if possible, a small project file.

## Security

Please report vulnerabilities (e.g. crafted project or audio files that crash or misbehave)
privately through GitHub's "Report a vulnerability" in the Security tab, not in a public issue.
