#!/bin/sh
# Packages the Electron contestant as a stock Electron.app with the app payload
# in Contents/Resources/app — the layout a production Electron build ships.
set -e
cd "$(dirname "$0")/.."
APP_DIR="../apps/electron"
DIST="$APP_DIR/node_modules/electron/dist/Electron.app"
OUT="$APP_DIR/build/BenchElectron.app"
# npm ci skips the electron postinstall when every package is cache-hit —
# node_modules exists but dist/ was never fetched; pull it explicitly
if [ ! -d "$DIST" ]; then
  node "$APP_DIR/node_modules/electron/install.js"
fi
if [ ! -d "$DIST" ]; then
  echo "electron dist/ still missing after install.js" >&2
  exit 1
fi
rm -rf "$OUT"
mkdir -p "$APP_DIR/build"
cp -R "$DIST" "$OUT"
mkdir -p "$OUT/Contents/Resources/app"
cp "$APP_DIR/package.json" "$APP_DIR/main.js" "$APP_DIR/renderer.js" "$APP_DIR/index.html" "$OUT/Contents/Resources/app/"
plutil -replace CFBundleIdentifier -string dev.bench.electron "$OUT/Contents/Info.plist"
plutil -replace CFBundleName -string BenchElectron "$OUT/Contents/Info.plist"
plutil -replace CFBundleExecutable -string Electron "$OUT/Contents/Info.plist" || true
# ad-hoc sign so it launches on this machine
codesign --force --deep --sign - "$OUT"
echo "built $OUT"
