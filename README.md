# deskbridge

Independent Deskflow-like bridge for a Windows server and a macOS client.

The first usable target is Windows controlling macOS over TCP:

- mouse sharing through a configured screen edge
- keyboard sharing with Windows scancodes mapped to macOS CoreGraphics events
- key repeat forwarding, including long-press Backspace
- PrintScreen mapped to macOS `Command-Control-Shift-4`
- bidirectional text clipboard
- bidirectional bitmap clipboard
- bidirectional file clipboard by transferring files first, then exposing local
  file paths to the receiving OS clipboard

## Build

On Windows:

```powershell
cd C:\Users\YOU\Downloads\deskbridge
cargo build --release
```

On macOS:

```bash
cd /path/to/deskbridge
cargo build --release
```

## Run

Start the server on Windows:

```powershell
.\target\release\deskbridge.exe server --bind 0.0.0.0:24920 --edge right
```

Start the client on macOS:

```bash
./target/release/deskbridge client --server WINDOWS_IP:24920
```

`--edge right` means macOS is on the right side of the Windows screen. Use
`--edge left` if macOS is on the left side.

The macOS client sends its main screen size during handshake, so the Windows
server does not need a manual macOS resolution argument.

## Clipboard

Text uses the native text formats on both systems.

Images are normalized as BMP on the wire. Windows reads and writes `CF_DIB`;
macOS reads and writes `com.microsoft.bmp` through `NSPasteboard`.

Files are sent as file-transfer frames. After all files arrive, the receiver
writes the received local paths into the platform clipboard:

- Windows uses `CF_HDROP`
- macOS uses file URLs on `NSPasteboard`

This means copying files in Explorer can become pasteable in Finder, and copying
files in Finder can become pasteable in Explorer.

## Control Notes

Move the Windows pointer through the configured edge to enter macOS control.
While controlling macOS, Windows mouse movement is treated as relative movement
from the center of the Windows screen, so the local screen edge does not limit
long mouse moves.

Move the macOS pointer back through the shared edge to release control back to
Windows. Press `ScrollLock` while controlling macOS to force release.

## Current Limit

Live drag-and-drop from Explorer directly into Finder is not implemented yet.
The reliable file path in this version is clipboard-based file transfer: copy
files on one side, then paste them on the other side.
