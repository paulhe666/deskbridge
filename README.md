# deskbridge

Independent Deskflow-like prototype for Windows server and macOS client.

This project intentionally does not reuse Deskflow's `DCLP` clipboard protocol
for files. Deskflow clipboard supports text, HTML, and bitmap; file movement
needs a first-class side channel so Windows can expose `CF_HDROP` and macOS can
expose file URLs after files are actually present locally.

## First Target

- Windows as server
- macOS as client
- text clipboard
- bitmap clipboard
- file clipboard
- keyboard/mouse input stream
- long-press repeat events
- drag file transfer across machines

## Current State

This is the application core skeleton:

- `protocol`: frames for input, clipboard, file transfer, and drag lifecycle
- `input/macos`: CoreGraphics input injection with the earlier Windows keyboard
  scancode to macOS keycode mapping
- `clipboard/macos`: text, BMP image, and file URL pasteboard support
- `file_transfer`: safe relative-path file streaming
- `server`: placeholder for Windows capture loop
- `client`: macOS receive/apply loop

The Windows server still needs the native capture loop implementation:

- low-level keyboard hook
- low-level mouse hook
- edge crossing detection
- cursor lock while controlling macOS
- Windows clipboard watcher for `CF_UNICODETEXT`, `CF_DIB`, and `CF_HDROP`
- file drag lifecycle mapping into `DragStart/DragUpdate/DragEnd`

## Run Shape

```bash
deskbridge server --bind 0.0.0.0:24920
deskbridge client --server WINDOWS_IP:24920
```

## Why This Shape

The transport is independent from Deskflow but keeps a similar split:

- platform input capture/injection
- clipboard platform adapters
- protocol frames
- file-transfer stream

That makes it possible to later wrap this in a GUI or port the pieces into a
Deskflow fork without mixing platform-specific code into the protocol layer.
