#!/usr/bin/env bash
set -euo pipefail

REPO="${REPO:-duxweb/dux-ai-node}"
VERSION="${VERSION:-latest}"
SERVER_URL="${SERVER_URL:-}"
CLIENT_NAME="${CLIENT_NAME:-}"
INSTALL_ROOT="${INSTALL_ROOT:-/opt/dux-ai-node}"
CONFIG_DIR="${CONFIG_DIR:-/etc/dux-ai-node}"
CONFIG_FILE="${CONFIG_FILE:-$CONFIG_DIR/config.toml}"
DATA_DIR="${DATA_DIR:-/var/lib/dux-ai-node/data}"
LOG_DIR="${LOG_DIR:-/var/log/dux-ai-node}"
CACHE_DIR="${CACHE_DIR:-/var/cache/dux-ai-node}"
SERVICE_NAME="${SERVICE_NAME:-dux-ai-node}"
RUN_USER="${RUN_USER:-dux-ai-node}"
SKIP_BROWSER=0
ACTION="install"

usage() {
  cat <<EOF
Usage: $0 [options]

Options:
  --version <tag>         Release tag, default: latest
  --repo <owner/name>     GitHub repo, default: duxweb/dux-ai-node
  --server-url <url>      Set node server_url after install
  --client-name <name>    Set node client_name after install
  --install-root <path>   Install root, default: /opt/dux-ai-node
  --config-dir <path>     Config dir, default: /etc/dux-ai-node
  --data-dir <path>       Data dir, default: /var/lib/dux-ai-node/data
  --log-dir <path>        Log dir, default: /var/log/dux-ai-node
  --cache-dir <path>      Cache dir, default: /var/cache/dux-ai-node
  --run-user <name>       Service user, default: dux-ai-node
  --skip-browser          Do not install headless Chromium
  -h, --help              Show this help

Examples:
  curl -fsSL https://raw.githubusercontent.com/duxweb/dux-ai-node/main/scripts/install-linux.sh | sudo bash -s -- --server-url http://duxai.test
  curl -fsSL https://raw.githubusercontent.com/duxweb/dux-ai-node/main/scripts/install-linux.sh | sudo VERSION=v0.1.0 bash -s -- --server-url https://ai.example.com
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      VERSION="$2"
      shift 2
      ;;
    --repo)
      REPO="$2"
      shift 2
      ;;
    --server-url)
      SERVER_URL="$2"
      shift 2
      ;;
    --client-name)
      CLIENT_NAME="$2"
      shift 2
      ;;
    --install-root)
      INSTALL_ROOT="$2"
      shift 2
      ;;
    --config-dir)
      CONFIG_DIR="$2"
      CONFIG_FILE="$CONFIG_DIR/config.toml"
      shift 2
      ;;
    --data-dir)
      DATA_DIR="$2"
      shift 2
      ;;
    --log-dir)
      LOG_DIR="$2"
      shift 2
      ;;
    --cache-dir)
      CACHE_DIR="$2"
      shift 2
      ;;
    --run-user)
      RUN_USER="$2"
      shift 2
      ;;
    --skip-browser)
      SKIP_BROWSER=1
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

if ! command -v apt-get >/dev/null 2>&1; then
  echo "Only Debian/Ubuntu with apt-get is supported by this installer" >&2
  exit 1
fi

source /etc/os-release
if [[ " ${ID:-} ${ID_LIKE:-} " != *" debian "* && " ${ID:-} ${ID_LIKE:-} " != *" ubuntu "* ]]; then
  echo "Only Debian/Ubuntu is supported by this installer" >&2
  exit 1
fi

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)
    TARGET="x86_64-unknown-linux-gnu"
    ;;
  aarch64|arm64)
    TARGET="aarch64-unknown-linux-gnu"
    ;;
  *)
    echo "Unsupported architecture: $ARCH" >&2
    exit 1
    ;;
esac

BASE_PACKAGES=(
  ca-certificates
  curl
  tar
  xz-utils
  systemd
  libxcb1
  libdbus-1-3
  libwayland-client0
  libxrandr2
  libpipewire-0.3-0
  libdrm2
  libgbm1
  libegl1
  libgl1
  libxkbcommon0
  fonts-noto-core
  fonts-noto-cjk
  fonts-noto-color-emoji
)

if [[ $SKIP_BROWSER -eq 0 ]]; then
  if apt-cache show chromium >/dev/null 2>&1; then
    BASE_PACKAGES+=(chromium)
  elif apt-cache show chromium-browser >/dev/null 2>&1; then
    BASE_PACKAGES+=(chromium-browser)
  else
    echo "Unable to find chromium package in apt sources" >&2
    exit 1
  fi
fi

apt-get update
apt-get install -y "${BASE_PACKAGES[@]}"

if ! id "$RUN_USER" >/dev/null 2>&1; then
  useradd --system --home-dir /var/lib/dux-ai-node --create-home --shell /usr/sbin/nologin "$RUN_USER"
fi

mkdir -p "$INSTALL_ROOT" "$CONFIG_DIR" "$DATA_DIR" "$LOG_DIR" "$CACHE_DIR"
chown -R "$RUN_USER":"$RUN_USER" /var/lib/dux-ai-node "$DATA_DIR" "$LOG_DIR" "$CACHE_DIR"

if [[ "$VERSION" == "latest" ]]; then
  PACKAGE_URL="https://github.com/$REPO/releases/latest/download/dux-ai-node-$TARGET.tar.gz"
else
  PACKAGE_URL="https://github.com/$REPO/releases/download/$VERSION/dux-ai-node-$TARGET.tar.gz"
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

CURRENT_BINARY="$INSTALL_ROOT/$TARGET/dux-ai-node-daemon"
CURRENT_VERSION=""
if [[ -x "$CURRENT_BINARY" ]]; then
  CURRENT_VERSION="$($CURRENT_BINARY --version 2>/dev/null | awk '{print $2}' | xargs || true)"
  ACTION="upgrade"
fi

if systemctl list-unit-files | grep -q "^$SERVICE_NAME.service"; then
  systemctl stop "$SERVICE_NAME" || true
fi

curl -fL "$PACKAGE_URL" -o "$TMP_DIR/dux-ai-node.tar.gz"
rm -rf "$INSTALL_ROOT/$TARGET"
tar -xzf "$TMP_DIR/dux-ai-node.tar.gz" -C "$INSTALL_ROOT"

BINARY="$INSTALL_ROOT/$TARGET/dux-ai-node-daemon"
if [[ ! -x "$BINARY" ]]; then
  echo "Installed binary not found: $BINARY" >&2
  exit 1
fi

TARGET_VERSION="$($BINARY --version 2>/dev/null | awk '{print $2}' | xargs || true)"
if [[ -n "$CURRENT_VERSION" && -n "$TARGET_VERSION" && "$CURRENT_VERSION" == "$TARGET_VERSION" ]]; then
  ACTION="reinstall"
fi

"$BINARY" --config "$CONFIG_FILE" init >/dev/null
if [[ -n "$SERVER_URL" ]]; then
  "$BINARY" --config "$CONFIG_FILE" config set server_url "$SERVER_URL" >/dev/null
fi
if [[ -n "$CLIENT_NAME" ]]; then
  "$BINARY" --config "$CONFIG_FILE" config set client_name "$CLIENT_NAME" >/dev/null
fi
"$BINARY" --config "$CONFIG_FILE" config set browser_preference auto >/dev/null
"$BINARY" --config "$CONFIG_FILE" config set browser_mode headless >/dev/null
"$BINARY" --config "$CONFIG_FILE" config set auto_connect true >/dev/null
"$BINARY" --config "$CONFIG_FILE" config set log_level info >/dev/null

cat > "/etc/systemd/system/$SERVICE_NAME.service" <<EOF
[Unit]
Description=Dux AI Node Daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$RUN_USER
Group=$RUN_USER
WorkingDirectory=$INSTALL_ROOT/$TARGET
ExecStart=$BINARY --config $CONFIG_FILE daemon
Restart=always
RestartSec=3
Environment=HOME=/var/lib/dux-ai-node
Environment=DUX_AI_NODE_CONFIG_DIR=$CONFIG_DIR
Environment=DUX_AI_NODE_CONFIG_FILE=$CONFIG_FILE
Environment=DUX_AI_NODE_DATA_DIR=$DATA_DIR
Environment=DUX_AI_NODE_LOG_DIR=$LOG_DIR
Environment=XDG_CACHE_HOME=$CACHE_DIR

[Install]
WantedBy=multi-user.target
EOF

chown root:root "$CONFIG_FILE" "/etc/systemd/system/$SERVICE_NAME.service"
chmod 644 "$CONFIG_FILE" "/etc/systemd/system/$SERVICE_NAME.service"

systemctl daemon-reload
systemctl enable --now "$SERVICE_NAME"

echo
echo "$(tr '[:lower:]' '[:upper:]' <<< "${ACTION:0:1}")${ACTION:1} Dux AI Node for $TARGET"
if [[ -n "$CURRENT_VERSION" ]]; then
  echo "Previous version: ${CURRENT_VERSION}"
fi
if [[ -n "$TARGET_VERSION" ]]; then
  echo "Current version:  ${TARGET_VERSION}"
fi
echo "Binary: $BINARY"
echo "Config: $CONFIG_FILE"
echo "Data:   $DATA_DIR"
echo "Logs:   $LOG_DIR"
echo
echo "Service commands:"
echo "  systemctl status $SERVICE_NAME"
echo "  journalctl -u $SERVICE_NAME -f"
echo
echo "Current config:"
cat "$CONFIG_FILE"
