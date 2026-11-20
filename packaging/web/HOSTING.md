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
- In the browser there are no plugins and no audio recording yet, and opening or saving files
  through the browser is still being worked on (issue #30).
