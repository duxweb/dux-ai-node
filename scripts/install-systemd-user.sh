#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UNIT_TEMPLATE="$ROOT/deploy/systemd/dux-ai-node.service"
TARGET_DIR="$HOME/.config/systemd/user"
TARGET_UNIT="$TARGET_DIR/dux-ai-node.service"
BIN_PATH="$ROOT/dist/linux/dux-ai-node-daemon"

if [ ! -x "$BIN_PATH" ]; then
  echo "Linux daemon binary not found: $BIN_PATH"
  echo "Run ./scripts/build-linux.sh first"
  exit 1
fi

mkdir -p "$TARGET_DIR"
sed "s|/usr/local/bin/dux-ai-node-daemon|$BIN_PATH|g" "$UNIT_TEMPLATE" > "$TARGET_UNIT"

systemctl --user daemon-reload
systemctl --user enable --now dux-ai-node.service
systemctl --user status --no-pager dux-ai-node.service || true

echo "Installed user service: $TARGET_UNIT"
