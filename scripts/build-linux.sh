#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_NAME="dux-ai-node-daemon"
TARGET="${TARGET:-$(rustc -vV | awk '/host:/ {print $2}')}"
TARGET="$(printf '%s' "$TARGET" | xargs)"
DIST_DIR="$ROOT/dist/linux/$TARGET"
HOST_OS="$(uname -s)"
BIN=""

if [[ "$TARGET" == *"linux"* ]]; then
  if [[ "$HOST_OS" != "Linux" && "$TARGET" == "x86_64-unknown-linux-gnu" ]]; then
    if ! command -v x86_64-linux-gnu-gcc >/dev/null 2>&1; then
      echo "Cross compiler missing: x86_64-linux-gnu-gcc"
      echo "On macOS, install a Linux cross toolchain first or build on a Linux host."
      echo "Then rerun: TARGET=$TARGET cargo build --release -p $BIN_NAME --target $TARGET"
      exit 1
    fi
  fi
  cargo build --release -p "$BIN_NAME" --target "$TARGET"
  BIN="$ROOT/target/$TARGET/release/$BIN_NAME"
else
  echo "Unsupported TARGET for build-linux.sh: $TARGET"
  exit 1
fi

rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"
cp "$BIN" "$DIST_DIR/$BIN_NAME"
mkdir -p "$DIST_DIR/web"
cp -R "$ROOT/web/settings" "$DIST_DIR/web/settings"
chmod +x "$DIST_DIR/$BIN_NAME"
( cd "$ROOT/dist/linux" && tar -czf "dux-ai-node-${TARGET}.tar.gz" "$TARGET" )

echo "Built Linux daemon: $DIST_DIR/$BIN_NAME"
