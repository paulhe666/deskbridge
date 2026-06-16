# Deskbridge

Deskbridge is a Deskflow-like bridge for Windows and macOS.

One binary contains:

- GUI control panel
- Windows server mode
- macOS client mode
- text, bitmap, and file clipboard sync
- low-latency keyboard and mouse forwarding
- tunable macOS wheel smoothing
- Windows edge drop strip for Explorer file drops
- native GUI file drop zone on both systems

## Install Packages

### macOS

Build the app and installer on macOS:

```bash
./scripts/package_macos.sh
```

Outputs:

- `dist/macos/Deskbridge.app`
- `dist/macos/Deskbridge-0.1.0.pkg`
- `dist/macos/Deskbridge-macOS-app.zip`
- `dist/macos/Uninstall Deskbridge.command`

Install by opening `Deskbridge-0.1.0.pkg`, or copy `Deskbridge.app` to
`/Applications`.

Uninstall by opening `Uninstall Deskbridge.command`.

### Windows

Build the app and installer on Windows PowerShell:

```powershell
.\scripts\package_windows.ps1
```

This requires Rust and Inno Setup 6. The installer is written to:

```text
dist\windows\Deskbridge-Setup-0.1.0.exe
```

The Windows installer creates a normal uninstall entry. You can also run:

```powershell
.\scripts\uninstall_windows.ps1
```

## GUI Usage

Launch `Deskbridge` directly.

On Windows:

1. Set role to `Server`.
2. Keep bind as `0.0.0.0:24920`.
3. Set edge to `right` if the Mac is to the right of the Windows screen.
4. Click `Start`.

On macOS:

1. Set role to `Client`.
2. Set server to `WINDOWS_IP:24920`.
3. Adjust scroll values if needed.
4. Click `Start`.

The GUI saves config to:

```text
~/.deskbridge/config.ini
```

## macOS Permissions

The macOS app needs Accessibility permission to inject keyboard and mouse input.

Open:

```text
System Settings -> Privacy & Security -> Accessibility
```

Add and enable `Deskbridge.app`, or the terminal app if running from terminal.

## File Transfer And Drag

File copy/paste works both ways:

- Explorer copy -> Finder paste
- Finder copy -> Explorer paste

GUI native drop works both ways:

- Drag files onto the Deskbridge GUI window.
- The app publishes the files to the local file clipboard.
- The running service sends them to the other computer.

Windows edge drop works one way:

- On Windows, drag Explorer files to the semi-transparent edge strip and release.
- The files are sent to macOS and written to the Finder pasteboard.

Current limit:

- Directly dragging a file across the screen edge and dropping into an arbitrary
  Finder or Explorer window is not fully implemented yet.
- That final behavior requires a deeper OLE/AppKit drag source bridge and needs
  real Windows and macOS interactive testing.

## Command Line

The GUI starts these same service commands internally.

Windows server:

```powershell
deskbridge.exe server --bind 0.0.0.0:24920 --edge right
```

macOS client:

```bash
deskbridge client --server WINDOWS_IP:24920
```

macOS client with scroll tuning:

```bash
DESKBRIDGE_SCROLL_SCALE=1.35 \
DESKBRIDGE_SCROLL_RESPONSE=0.38 \
DESKBRIDGE_SCROLL_MAX_STEP=120 \
DESKBRIDGE_SCROLL_FRAME_MS=8 \
deskbridge client --server WINDOWS_IP:24920
```

## Scroll Tuning

Useful values:

- `DESKBRIDGE_SCROLL_SCALE`: total distance per wheel detent
- `DESKBRIDGE_SCROLL_RESPONSE`: how quickly pending scroll distance is released
- `DESKBRIDGE_SCROLL_MAX_STEP`: max pixels per animation frame
- `DESKBRIDGE_SCROLL_FRAME_MS`: scroll animation frame interval

For a tactile wheel, increase `distance` first. If fast multi-notch scrolling
feels delayed, increase `response`. If pages jump too sharply, lower `max step`.

## Low Latency Notes

Use a direct LAN IP when possible. Avoid relay paths for keyboard and mouse.

Windows input hooks queue events to a sender thread instead of writing to TCP in
the hook callback. Mouse movement and wheel deltas are coalesced at a 4ms flush
interval; keyboard and button events flush immediately.
