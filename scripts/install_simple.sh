#!/usr/bin/env bash
# RamShield Quick-Start Installer
# curl -fsSL https://ramshield.io/install.sh | bash
# Installs safe-mode (no XDP) runtime; sets up systemd service.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

# Detect platform
ARCH=$(uname -m)
case "$ARCH" in
  x86_64|amd64) TARGET=x86_64-unknown-linux-gnu ;;
  aarch64|arm64) TARGET=aarch64-unknown-linux-gnu ;;
  *) die "unsupported architecture: $ARCH" ;;
esac

# Fetch latest release info from GitHub API (no VERSION pin required)
REPO="grep999/ramshield"
API="https://api.github.com/repos/$REPO/releases/latest"
LATEST=$(curl -fsSL --retry 5 -H "Accept: application/vnd.github.v3+json" "$API" | python3 -c "import sys,json; print(json.load(sys.stdin)['tag_name'].lstrip('v'))" 2>/dev/null || echo "0.1.0")
VERSION="${VERSION:-$LATEST}"

if [ -z "$VERSION" ] || [ "$VERSION" = "null" ]; then
  die "cannot determine latest version; set VERSION=x.y.z or check network"
fi

# Download binary
BINARY="ramshield-${VERSION}-${TARGET}.tar.gz"
BASE="https://github.com/grep999/ramshield/releases/download/v${VERSION}"
curl -fsSL --retry 5 "$BASE/$BINARY" -o /tmp/ramshield.tar.gz || die "failed to download binary"

# Extract and install
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
tar -xzf /tmp/ramshield.tar.gz -C "$TMP"
BIN="$TMP/ramshield"
[ -x "$BIN" ] || die "downloaded binary not executable"

PREFIX="${PREFIX:-/usr/local}"
mkdir -p "$PREFIX/bin"
cp "$BIN" "$PREFIX/bin/ramshield"
chmod 0755 "$PREFIX/bin/ramshield"

# Safe-mode config (no XDP, userspace only)
CONFIG_DIR=/etc/ramshield
STATE_DIR=/var/lib/ramshield
mkdir -p "$CONFIG_DIR" "$STATE_DIR" "$STATE_DIR/wal" "$STATE_DIR/snapshots"
cp "$ROOT/config-min.toml" "$CONFIG_DIR/config.toml"
chown root:ramshield "$CONFIG_DIR/config.toml"; chmod 0640 "$CONFIG_DIR/config.toml"

# Create systemd service (userspace only, no XDP)
cat > /etc/systemd/system/ramshield.service <<UNIT
[Unit]
Description=RamShield Ingress Defense (safe mode: userspace, no XDP)
After=network-online.target
Wants=network-online.target
Documentation=https://github.com/grep999/ramshield

[Service]
Type=simple
User=ramshield
Group=ramshield
ExecStart=/usr/local/bin/ramshield --config /etc/ramshield/config.toml
Restart=on-failure
RestartSec=5s
StartLimitIntervalSec=60
StartLimitBurst=5
LimitNOFILE=65535
NoNewPrivileges=true
PrivateTmp=true
PrivateDevices=true
ProtectSystem=strict
ProtectHome=true
ProtectControlGroups=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectKernelLogs=true
RestrictRealtime=true
RestrictSUIDSGID=true
LockPersonality=true
MemoryDenyWriteExecute=true
CapabilityBoundingSet=CAP_NET_ADMIN CAP_BPF CAP_PERFMON CAP_NET_RAW
AmbientCapabilities=CAP_NET_ADMIN CAP_BPF CAP_PERFMON CAP_NET_RAW
ReadWritePaths=/var/lib/ramshield /dev/shm /sys/fs/bpf

[Install]
WantedBy=multi-user.target
UNIT

# Create user/group if needed
if ! id -u ramshield >/dev/null 2>&1; then
  useradd --system --home /nonexistent --shell /usr/sbin/nologin ramshield
fi
getent group ramshield >/dev/null 2>&1 || groupadd --system ramshield
usermod -g ramshield ramshield

# Set permissions
install -d -o root -g ramshield -m 0750 "$CONFIG_DIR"
install -d -o ramshield -g ramshield -m 0750 "$STATE_DIR"
install -d -o ramshield -g ramshield -m 0750 "$STATE_DIR/wal" "$STATE_DIR/snapshots"

# Set up firewall-friendly iptables rules (optional, best-effort)
iptables -A INPUT -p tcp --dport 7890 -j ACCEPT 2>/dev/null || true

log "installed v$VERSION (safe mode: userspace, no XDP)"
log "config: $CONFIG_DIR/config.toml"
log "start: systemctl enable --now ramshield.service"
