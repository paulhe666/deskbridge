#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist/macos"

cd "$ROOT"
cargo build --release

if [ -d "$DIST" ]; then
  chmod -R u+w "$DIST" || true
fi
if ! rm -rf "$DIST" 2>/dev/null; then
  DIST="$ROOT/dist/macos-$(date +%Y%m%d%H%M%S)"
fi

APP="$DIST/Deskbridge.app"
BIN="$APP/Contents/MacOS/deskbridge"
RES="$APP/Contents/Resources"

mkdir -p "$(dirname "$BIN")" "$RES"
cp "$ROOT/target/release/deskbridge" "$BIN"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>deskbridge</string>
  <key>CFBundleIdentifier</key>
  <string>local.deskbridge.app</string>
  <key>CFBundleName</key>
  <string>Deskbridge</string>
  <key>CFBundleDisplayName</key>
  <string>Deskbridge</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>0.1.0</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST

cat > "$DIST/Uninstall Deskbridge.command" <<'UNINSTALL'
#!/usr/bin/env bash
set -euo pipefail
rm -rf /Applications/Deskbridge.app
rm -rf "$HOME/.deskbridge"
echo "Deskbridge has been removed."
read -r -p "Press Enter to close this window."
UNINSTALL
chmod +x "$DIST/Uninstall Deskbridge.command"

if command -v pkgbuild >/dev/null 2>&1; then
  pkgbuild \
    --component "$APP" \
    --install-location /Applications \
    --identifier local.deskbridge.app \
    --version 0.1.0 \
    "$DIST/Deskbridge-0.1.0.pkg"
fi

ditto -c -k --keepParent "$APP" "$DIST/Deskbridge-macOS-app.zip"

echo "Created:"
find "$DIST" -maxdepth 1 -type f -print
