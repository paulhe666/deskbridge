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
- Windows edge drop strip for dragging Explorer files to macOS
- low-latency input queue with mouse/wheel coalescing outside the Windows hook

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

## Low Latency

The Windows input hook never writes to the TCP stream directly. It queues input
events to a sender thread, which flushes mouse motion and wheel deltas every 4ms.
Keyboard and button events flush immediately.

For the best connection, use the direct LAN IP instead of a relay address. Both
ends set `TCP_NODELAY`, so packet coalescing should not add latency.

## Scrolling

The macOS client turns low-resolution wheel detents into a smooth stream of
pixel scroll events. The defaults are tuned for a tactile wheel, but they can be
changed without rebuilding:

```bash
DESKBRIDGE_SCROLL_SCALE=1.35 \
DESKBRIDGE_SCROLL_RESPONSE=0.38 \
DESKBRIDGE_SCROLL_MAX_STEP=120 \
./target/release/deskbridge client --server WINDOWS_IP:24920
```

Useful knobs:

- `DESKBRIDGE_SCROLL_SCALE`: total distance per wheel detent
- `DESKBRIDGE_SCROLL_RESPONSE`: how quickly pending scroll distance is released
- `DESKBRIDGE_SCROLL_MAX_STEP`: max pixels per animation frame
- `DESKBRIDGE_SCROLL_FRAME_MS`: scroll animation frame interval

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

## File Drag

The Windows server creates a narrow, semi-transparent drop strip on the shared
screen edge. Drag files from Explorer to that strip and release. The server sends
the files to macOS, and the macOS client writes the received file URLs to the
Finder pasteboard.

The strip is only used while dragging with the left button held down. Normal
mouse movement through the same edge still enters macOS control.

## Control Notes

Move the Windows pointer through the configured edge to enter macOS control.
While controlling macOS, Windows mouse movement is treated as relative movement
from the center of the Windows screen, so the local screen edge does not limit
long mouse moves.

Move the macOS pointer back through the shared edge to release control back to
Windows. Press `ScrollLock` while controlling macOS to force release.

## Current Limit

Windows-to-macOS file drag is edge-strip based, not direct placement into the
current Finder window. macOS-to-Windows live drag still uses file clipboard
transfer in this version.
