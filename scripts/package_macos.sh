#!/usr/bin/env bash
set -euo pipefail
export COPYFILE_DISABLE=1

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$ROOT/dist/macos"
VERSION="${DESKBRIDGE_VERSION:-$(awk -F '"' '/^version = / { print $2; exit }' "$ROOT/Cargo.toml")}"
RELEASE_DIST="$ROOT/dist/releases/$VERSION/macos"
TAURI_CLI="$ROOT/web/node_modules/.bin/tauri"
TAURI_APP="$ROOT/target/release/bundle/macos/Deskbridge.app"

cd "$ROOT"

if [ -f "$ROOT/web/package.json" ]; then
  npm --prefix "$ROOT/web" install
fi

if [ ! -x "$TAURI_CLI" ]; then
  echo "Missing Tauri CLI at $TAURI_CLI" >&2
  echo "Run: npm --prefix web install" >&2
  exit 1
fi

npm --prefix "$ROOT/web" run build

"$TAURI_CLI" build --bundles app --config '{"build":{"beforeBuildCommand":""}}'

if [ ! -d "$TAURI_APP" ]; then
  echo "Tauri did not create expected app bundle: $TAURI_APP" >&2
  exit 1
fi

if [ -d "$DIST" ]; then
  chmod -R u+w "$DIST" || true
fi
if ! rm -rf "$DIST" 2>/dev/null; then
  DIST="$ROOT/dist/macos-$(date +%Y%m%d%H%M%S)"
fi
mkdir -p "$DIST"

APP="$DIST/Deskbridge.app"
ditto "$TAURI_APP" "$APP"

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
  PKGBUILD_WORK="$DIST/pkgbuild-work"
  PKG_ROOT="$PKGBUILD_WORK/root"
  COMPONENT_PLIST="$PKGBUILD_WORK/Deskbridge-component.plist"
  rm -rf "$PKGBUILD_WORK"
  mkdir -p "$PKG_ROOT/Applications"
  ditto "$APP" "$PKG_ROOT/Applications/Deskbridge.app"
  find "$PKG_ROOT" -name '._*' -delete
  cat > "$COMPONENT_PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array>
  <dict>
    <key>BundleHasStrictIdentifier</key>
    <true/>
    <key>BundleIsRelocatable</key>
    <false/>
    <key>BundleIsVersionChecked</key>
    <true/>
    <key>BundleOverwriteAction</key>
    <string>upgrade</string>
    <key>RootRelativeBundlePath</key>
    <string>Applications/Deskbridge.app</string>
  </dict>
</array>
</plist>
PLIST
  pkgbuild \
    --root "$PKG_ROOT" \
    --component-plist "$COMPONENT_PLIST" \
    --install-location / \
    --identifier local.deskbridge.app \
    --version "$VERSION" \
    "$DIST/Deskbridge-$VERSION.pkg"
  rm -rf "$PKGBUILD_WORK"

  if [ -n "${MACOS_INSTALLER_IDENTITY:-}" ]; then
    productsign \
      --sign "$MACOS_INSTALLER_IDENTITY" \
      "$DIST/Deskbridge-$VERSION.pkg" \
      "$DIST/Deskbridge-$VERSION-signed.pkg"
    mv "$DIST/Deskbridge-$VERSION-signed.pkg" "$DIST/Deskbridge-$VERSION.pkg"
  fi
fi

ditto -c -k --keepParent "$APP" "$DIST/Deskbridge-macOS-app.zip"

if [ -d "$RELEASE_DIST" ]; then
  chmod -R u+w "$RELEASE_DIST" || true
  rm -rf "$RELEASE_DIST"
fi
mkdir -p "$RELEASE_DIST"
find "$DIST" -maxdepth 1 -type f -exec cp {} "$RELEASE_DIST" \;

echo "Created:"
find "$DIST" -maxdepth 1 -type f -print

echo "Archived release files:"
find "$RELEASE_DIST" -maxdepth 1 -type f -print
