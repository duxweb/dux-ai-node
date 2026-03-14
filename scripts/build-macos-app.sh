#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_NAME="Dux AI Node"
APP_DIR="$ROOT/dist/${APP_NAME}.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"
BIN="$ROOT/target/release/dux-ai-node"
AX_HELPER_DIR="$ROOT/helpers/macos-ax-helper"
AX_HELPER_BIN="$AX_HELPER_DIR/.build/release/dux-node-macos-ax-helper"
ICON="$ROOT/assets/icon.icns"
VERSION="${APP_VERSION:-0.1.1}"
ARCH="$(uname -m)"
SWIFT_BIN="$(command -v swift || true)"

if [[ ! -f "$ICON" ]]; then
  echo "Missing app icon: $ICON"
  exit 1
fi
if [[ -z "$SWIFT_BIN" || ! -x "$SWIFT_BIN" ]]; then
  echo "Missing swift binary in PATH. Install Xcode command line tools first."
  exit 1
fi

mkdir -p "$ROOT/dist"
cargo build --release -p dux-ai-node
swift build --configuration release --package-path "$AX_HELPER_DIR"
rm -rf "$APP_DIR"
RUNTIME_HELPER_DIR="$RESOURCES_DIR/runtime/helpers"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR" "$RUNTIME_HELPER_DIR"
cp "$BIN" "$MACOS_DIR/dux-ai-node"
cp "$AX_HELPER_BIN" "$RUNTIME_HELPER_DIR/dux-node-macos-ax-helper"
chmod +x "$RUNTIME_HELPER_DIR/dux-node-macos-ax-helper"
cp "$ICON" "$RESOURCES_DIR/icon.icns"
mkdir -p "$RESOURCES_DIR/web"
cp -R "$ROOT/web/settings" "$RESOURCES_DIR/web/settings"
cat > "$CONTENTS_DIR/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>zh_CN</string>
  <key>CFBundleExecutable</key>
  <string>dux-ai-node</string>
  <key>CFBundleIdentifier</key>
  <string>plus.dux.ai.node</string>
  <key>CFBundleIconFile</key>
  <string>icon.icns</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Dux AI Node</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${VERSION}</string>
  <key>LSUIElement</key>
  <true/>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST

( cd "$ROOT/dist" && zip -qry "Dux-AI-Node-macos-${ARCH}.zip" "${APP_NAME}.app" )

echo "Built app: $APP_DIR"
