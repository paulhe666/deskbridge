#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist/macos"
VERSION="$(awk -F '"' '/^version = / { print $2; exit }' "$ROOT/Cargo.toml")"

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
cp "$ROOT/assets/deskbridge.icns" "$RES/Deskbridge.icns"

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
  <key>CFBundleIconFile</key>
  <string>Deskbridge</string>
  <key>CFBundleShortVersionString</key>
  <string>__VERSION__</string>
  <key>CFBundleVersion</key>
  <string>__VERSION__</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST
perl -0pi -e "s/__VERSION__/$VERSION/g" "$APP/Contents/Info.plist"

APP_SIGN_IDENTITY="${MACOS_APPLICATION_IDENTITY:--}"
codesign --force --deep --options runtime --sign "$APP_SIGN_IDENTITY" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

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
    --version "$VERSION" \
    "$DIST/Deskbridge-$VERSION.pkg"

  if [ -n "${MACOS_INSTALLER_IDENTITY:-}" ]; then
    productsign \
      --sign "$MACOS_INSTALLER_IDENTITY" \
      "$DIST/Deskbridge-$VERSION.pkg" \
      "$DIST/Deskbridge-$VERSION-signed.pkg"
    mv "$DIST/Deskbridge-$VERSION-signed.pkg" "$DIST/Deskbridge-$VERSION.pkg"
  fi
fi

ditto -c -k --keepParent "$APP" "$DIST/Deskbridge-macOS-app.zip"

echo "Created:"
find "$DIST" -maxdepth 1 -type f -print
