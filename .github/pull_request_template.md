## What and why

<!-- What does this change, and which issue does it address? (Fixes #…) -->

## How it was tested

- [ ] `cargo xtask ci` passes locally
- [ ] New or changed behaviour has tests
- [ ] UI change: screenshots (dark and light) attached

<!-- What did you run? What could you not test (OS, audio device, MIDI, plugins)? -->

## Checklist

- [ ] Clean room: nothing copied from other DAWs (proprietary or GPL)
- [ ] New dependencies/assets are permissively licensed and credited
- [ ] No new `unwrap`/`expect`/`panic!` outside tests; no allocation/locks on the audio thread
