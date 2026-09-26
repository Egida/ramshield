#!/usr/bin/env bash
# install.sh — one-command RamShield install for Linux hosts.
# Usage: curl -fsSL https://github.com/grep999/ramshield/raw/master/scripts/install.sh | sh
# Requires: systemd, cargo/nightly (for source build) OR curl/docker (for binary/image).

set -euo pipefail

INSTALL_MODE="${INSTALL_MODE:-auto}"  # auto | binary | docker | source
INSTALL_PREFIX="/usr/local"
CONFIG_DIR="/etc/ramshield"
WAL_DIR="/var/lib/ramshield"
SERVICE_USER="ramshield"
SERVICE_GROUP="ramshield"

log()  { printf '\033[32m[install]\033[0m %s\n' "$*"; }
warn() { printf '\033[33m[install]\033[0m %s\n' "$*"; }
err()  { printf '\033[31m[install]\033[0m %s\n' "$*" >&2; }

need_cmd() { command -v "$1" >/dev/null 2>&1 || { err "missing: $1"; exit 1; }; }

detect_mode() {
    if command -v docker >/dev/null 2>&1; then echo "docker"; return; fi
    if command -v cargo >/dev/null 2>&1; then echo "source"; return; fi
    echo "binary"
}

create_user() {
    id -u "$SERVICE_USER" >/dev/null 2>&1 || {
        log "creating service user $SERVICE_USER"
        useradd -r -s /usr/sbin/nologin -d /nonexistent -c "RamShield daemon" "$SERVICE_USER"
    }
}

setup_dirs() {
    mkdir -p "$CONFIG_DIR" "$WAL_DIR"
    chown "$SERVICE_USER:$SERVICE_GROUP" "$WAL_DIR"
    chmod 750 "$WAL_DIR"
}

install_binary() {
    local url="https://github.com/grep999/ramshield/releases/latest/download/ramshield-linux-amd64"
    log "downloading binary from $url"
    curl -fsSL "$url" -o /tmp/ramshield
    chmod +x /tmp/ramshield
    install -o root -g root -m 0755 /tmp/ramshield "$INSTALL_PREFIX/bin/ramshield"
    # File capabilities for XDP (re-apply after every rebuild)
    setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' "$INSTALL_PREFIX/bin/ramshield" 2>/dev/null || warn "setcap failed (run as root or skip if not using XDP)"
}

install_docker() {
    log "pulling ghcr.io/grep999/ramshield:latest"
    docker pull ghcr.io/grep999/ramshield:latest
}

install_source() {
    need_cmd cargo
    need_cmd rustup
    log "building from source (nightly-2026-08-29)"
    rustup toolchain install nightly-2026-08-29
    rustup default nightly-2026-08-29
    cd /tmp
    git clone --depth 1 --branch master https://github.com/grep999/ramshield.git
    cd ramshield/beta/rs
    cargo build --release --locked -F full
    strip target/release/ramshield
    install -o root -g root -m 0755 target/release/ramshield "$INSTALL_PREFIX/bin/ramshield"
    setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' "$INSTALL_PREFIX/bin/ramshield" 2>/dev/null || warn "setcap failed"
}

install_config() {
    if [[ ! -f "$CONFIG_DIR/config.toml" ]]; then
        log "installing config template to $CONFIG_DIR/config.toml"
        cat > "$CONFIG_DIR/config.toml" <<'EOF'
# RamShield production config — EDIT before starting
# Full docs: https://github.com/grep999/ramshield/blob/master/docs/TUNING.md

[engine]
shard_count = 256
worker_threads = 0
ram_limit_mb = 14512

[ipc]
tcp_addr = "127.0.0.1:7890"
max_connections = 1000000
max_line_length = 33554432
# auth_keys = ["k1:<hex>"]  # required for production
# key_roles = [{ key_id = "k1", role = "Admin" }]

[dashboard]
enabled = true
http_addr = "127.0.0.1:9999"
max_login_attempts = 50
max_password_length = 1024
# admin_password_hash = "<argon2>"  # set in production

[wal]
enabled = true
dir = "/var/lib/ramshield"
durability = "GroupCommit"
compress = false
seg_max_bytes = 67108864
retention_max_bytes = 1073741824

[forecasting]
enabled = true
ewma_alpha = 0.3
hw_beta = 0.1
hw_gamma = 0.1
seasonality_period = 60
anomaly_zscore = 3.0
min_entropy = 4.5

[detection]
rps_threshold = 500
rate_window_secs = 10
subnet_batch_threshold = 64
subnet_batch_min_events = 500
batch_block_enabled = true
block_ttl_secs = 300
pulse_window_secs = 5
pulse_threshold_samples = 3
subnet_burst_ttl_secs = 60
bloom_bits = 8000000
batch_max_events = 4096
batch_window_ms = 25
pre_aggs_flush_interval_ms = 250
promote_min_events = 4
subnet_window_threshold = 12
emergency_burst_threshold = 500
pre_aggs_max_size = 1000000

[xdp]
enabled = true
interface = "eth0"   # CHANGE to your interface
mode = "skb"
EOF
    else
        log "config exists at $CONFIG_DIR/config.toml — leaving unchanged"
    fi
}

install_systemd() {
    log "installing systemd unit"
    cat > /etc/systemd/system/ramshield.service <<'EOF'
[Unit]
Description=RamShield Autonomous Ingress Defense Daemon
Documentation=https://github.com/grep999/ramshield
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/ramshield --config /etc/ramshield/config.toml
Restart=on-failure
RestartSec=5s
LimitNOFILE=65535

User=ramshield
Group=ramshield

AmbientCapabilities=CAP_NET_ADMIN CAP_BPF CAP_PERFMON
CapabilityBoundingSet=CAP_NET_ADMIN CAP_BPF CAP_PERFMON

NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
PrivateDevices=true
ProtectControlGroups=true
ProtectKernelTunables=true
ProtectKernelModules=true
ReadWritePaths=/var/lib/ramshield /dev/shm /sys/fs/bpf

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
}

main() {
    log "RamShield installer starting (mode: $INSTALL_MODE)"
    [[ $EUID -eq 0 ]] || { err "run as root"; exit 1; }

    case "$INSTALL_MODE" in
        auto) INSTALL_MODE=$(detect_mode) ;;
        binary|docker|source) ;;
        *) err "unknown INSTALL_MODE: $INSTALL_MODE"; exit 1 ;;
    esac

    create_user
    setup_dirs

    case "$INSTALL_MODE" in
        binary) install_binary ;;
        docker) install_docker ;;
        source) install_source ;;
    esac

    install_config
    install_systemd

    log "done. Next steps:"
    echo "  1. Edit /etc/ramshield/config.toml (set auth_keys, key_roles, xdp.interface, admin_password_hash)"
    echo "  2. systemctl enable --now ramshield"
    echo "  3. ramshield doctor  # verify readiness"
    echo "  4. journalctl -u ramshield -f"
}

main "$@"