#!/usr/bin/env bash
set -euo pipefail

INSTALL_ROOT="${INSTALL_ROOT:-/opt/dux-ai-node}"
CONFIG_DIR="${CONFIG_DIR:-/etc/dux-ai-node}"
DATA_DIR="${DATA_DIR:-/var/lib/dux-ai-node/data}"
LOG_DIR="${LOG_DIR:-/var/log/dux-ai-node}"
CACHE_DIR="${CACHE_DIR:-/var/cache/dux-ai-node}"
SERVICE_NAME="${SERVICE_NAME:-dux-ai-node}"
RUN_USER="${RUN_USER:-dux-ai-node}"
PURGE=0

usage() {
  cat <<EOF
Usage: $0 [options]

Options:
  --purge     Remove config, data, logs and cache directories
  -h, --help  Show this help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --purge)
      PURGE=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage
      exit 1
      ;;
  esac
done

if [[ $(id -u) -ne 0 ]]; then
  echo "Please run as root" >&2
  exit 1
fi

if systemctl list-unit-files | grep -q "^$SERVICE_NAME.service"; then
  systemctl disable --now "$SERVICE_NAME" || true
  rm -f "/etc/systemd/system/$SERVICE_NAME.service"
  systemctl daemon-reload
fi

rm -rf "$INSTALL_ROOT"

if [[ $PURGE -eq 1 ]]; then
  rm -rf "$CONFIG_DIR" "$DATA_DIR" "$LOG_DIR" "$CACHE_DIR"
  if id "$RUN_USER" >/dev/null 2>&1; then
    userdel "$RUN_USER" || true
  fi
fi

echo "Uninstalled Dux AI Node"
if [[ $PURGE -eq 1 ]]; then
  echo "Config, data, logs and cache were removed"
else
  echo "Config and data were kept. Re-run with --purge to remove all state"
fi
