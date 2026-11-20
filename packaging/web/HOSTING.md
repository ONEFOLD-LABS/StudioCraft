# Hosting the StudioCraft web build

This folder is a static site. Serve it over HTTP(S) from any path, for example:

```sh
python3 -m http.server 8080        # then open http://localhost:8080/
```

- `.wasm` files must be served with the MIME type `application/wasm`. Most servers do this by
  default.
- The page draws with WebGPU and falls back to WebGL2.
- Browsers allow sound only after a user gesture: StudioCraft starts audio output at the first
  click or key press in the window.
- Files go through the browser: *Speichern* downloads the project as `Name.studiocraft.zip`
  (the project folder with its audio), *Öffnen* takes such a zip or a single `.studiocraft`
  file, *Audio importieren* picks audio files and the mixdown export downloads a WAV.
- In the browser there are no plugins, no audio recording, no MIDI and no autosave.
