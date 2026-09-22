#!/usr/bin/env bash
# RamShield installer — single binary, systemd service, config
# Usage: curl -fsSL https://get.ramshield.dev | bash
#    or: bash install.sh [--version 0.2.0] [--prefix /usr/local]
set -euo pipefail

VERSION="${VERSION:-0.2.0}"
PREFIX="${PREFIX:-/usr/local}"
CONFIG_DIR="${CONFIG_DIR:-$HOME/.config/ramshield}"
REPO="grep999/ramshield"

RED='\033[0;31m'
GREEN='\033[0;32m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${GREEN}[*]${NC} $*"; }
error() { echo -e "${RED}[!]${NC} $*" >&2; }
die()   { error "$*"; exit 1; }

check_deps() {
    local deps=(curl tar sha256sum)
    for d in "${deps[@]}"; do
        command -v "$d" >/dev/null 2>&1 || die "Missing: $d"
    done
}

detect_arch() {
    local arch
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64)   echo "x86_64-unknown-linux-gnu" ;;
        aarch64|arm64)  echo "aarch64-unknown-linux-gnu" ;;
        armv7l|armhf)   echo "armv7-unknown-linux-gnueabihf" ;;
        *) die "Unsupported architecture: $arch" ;;
    esac
}

detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "macos" ;;
        *)       die "Unsupported OS: $(uname -s)" ;;
    esac
}

download_binary() {
    local os arch target url
    os=$(detect_os)
    arch=$(detect_arch)
    target="${arch}"

    if [ "$os" = "macos" ]; then
        target="apple-darwin"
    fi

    url="https://github.com/${REPO}/releases/download/v${VERSION}/ramshield-${target}.tar.gz"
    info "Downloading RamShield v${VERSION} (${target})..."
    
    local tmpdir
    tmpdir=$(mktemp -d)
    trap "rm -rf $tmpdir" EXIT

    curl -fsSL "$url" -o "${tmpdir}/ramshield.tar.gz" || {
        error "Download failed. Check https://github.com/${REPO}/releases"
        error "URL: $url"
        exit 1
    }

    info "Extracting..."
    tar -xzf "${tmpdir}/ramshield.tar.gz" -C "${tmpdir}"

    info "Installing to ${PREFIX}/bin/..."
    sudo install -m 755 "${tmpdir}/ramshield" "${PREFIX}/bin/ramshield" 2>/dev/null \
        || install -m 755 "${tmpdir}/ramshield" "${PREFIX}/bin/ramshield"

    info "Installed: $(command -v ramshield || echo "${PREFIX}/bin/ramshield")"
}

install_config() {
    if [ -f "${CONFIG_DIR}/config.toml" ]; then
        info "Config exists at ${CONFIG_DIR}/config.toml — skipping"
        return
    fi

    mkdir -p "${CONFIG_DIR}"
    # Ship the prod-verified baseline (single config of record in the repo),
    # pinned to this release tag so a trial behaves exactly like a prod run.
    # Set a dashboard admin password via env at first start:
    #   RAMSHIELD_DASHBOARD__ADMIN_PASSWORD='your-secret' ramshield --config config.toml
    if ! curl -fsSL "https://raw.githubusercontent.com/${REPO}/v${VERSION}/config.baseline.toml" -o "${CONFIG_DIR}/config.toml"; then
        error "Could not fetch baseline config for v${VERSION}"
        exit 1
    fi

    info "Config written to ${CONFIG_DIR}/config.toml"
    info "IMPORTANT: Set admin password via RAMSHIELD_DASHBOARD__ADMIN_PASSWORD env or admin_password_hash in config.toml"
}

install_systemd() {
    if [ "$(id -u)" -ne 0 ] || [ ! -d /etc/systemd/system ]; then
        return
    fi

    if [ -f /etc/systemd/system/ramshield.service ]; then
        info "systemd service exists — skipping"
        return
    fi

    cat > /etc/systemd/system/ramshield.service << EOF
[Unit]
Description=RamShield DDoS Detection Daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=${PREFIX}/bin/ramshield --config ${CONFIG_DIR}/config.toml
Restart=on-failure
RestartSec=5
LimitNOFILE=65536

# Security hardening
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=${CONFIG_DIR}

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    systemctl enable ramshield.service
    info "systemd service installed and enabled"
}

verify_install() {
    info "Verifying installation..."
    
    if command -v ramshield >/dev/null 2>&1; then
        local ver
        ver=$(ramshield --version 2>/dev/null || echo "unknown")
        info "RamShield installed: ${ver}"
    else
        die "Installation failed — ramshield not in PATH"
    fi
}

print_next_steps() {
    echo ""
    echo -e "${BOLD}RamShield v${VERSION} installed!${NC}"
    echo ""
    echo "Next steps:"
    echo "  1. Edit config:     ${CONFIG_DIR}/config.toml"
    echo "  2. Start daemon:    ramshield --config ${CONFIG_DIR}/config.toml"
    echo "  3. Open dashboard:  http://localhost:9999"
    echo "  4. IPC port:        localhost:7890 (newline-delimited JSON)"
    echo ""
    echo "Quick test:"
    echo "  ramshield --config ${CONFIG_DIR}/config.toml &"
    echo "  curl http://localhost:9999/healthz"
    echo ""
    echo "Docs: https://github.com/grep999/ramshield"
}

main() {
    echo -e "${BOLD}RamShield Installer${NC}"
    echo "DDoS detection engine — single binary, zero dependencies"
    echo ""

    check_deps
    download_binary
    install_config
    install_systemd
    verify_install
    print_next_steps
}

main "$@"
