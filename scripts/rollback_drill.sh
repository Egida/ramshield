#!/usr/bin/env bash
# Rollback drill: previous digest -> healthz
# Usage: bash scripts/rollback_drill.sh [--version 0.1.9] [--prefix /usr/local]
#
# Does NOT require a release artifact; it downloads the previous tagged
# release from GitHub, verifies its checksum, installs it, restarts the
# systemd unit (or the running binary if started manually), and polls /healthz.
#
# This is a local-only safety drill. It does not affect git history.

set -euo pipefail

REPO="grep999/ramshield"
PREFIX="${PREFIX:-/usr/local}"
CONFIG_DIR="${CONFIG_DIR:-$HOME/.config/ramshield}"
VERSION=""
HEALTHZ_URL="http://localhost:9999/healthz"
POLL_TIMEOUT=30

info()  { echo "[*] $*"; }
error() { echo "[!] $*" >&2; }
die()   { error "$*"; exit 1; }

usage() {
    cat <<'EOF'
Rollback drill for RamShield.
  --version V    Target version to roll back to (e.g. 0.1.9). Default: previous tag.
  --prefix P     Install prefix (default: /usr/local)
  --config C     Config directory (default: ~/.config/ramshield)
  --healthz U    Health endpoint URL (default: http://localhost:9999/healthz)
  --timeout S    Poll timeout seconds (default: 30)
EOF
}

# Parse args
while [[ $# -gt 0 ]]; do
    case $1 in
        --version) VERSION="$2"; shift 2 ;;
        --prefix)  PREFIX="$2"; shift 2 ;;
        --config)  CONFIG_DIR="$2"; shift 2 ;;
        --healthz) HEALTHZ_URL="$2"; shift 2 ;;
        --timeout) POLL_TIMEOUT="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) die "Unknown option: $1" ;;
    esac
done

# Resolve previous version if not given
if [[ -z "$VERSION" ]]; then
    info "Discovering previous release tag..."
    if ! VERSION=$(git -C "$(dirname "$0")/.." describe --tags --abbrev=0 2>/dev/null); then
        # Fallback: fetch from GitHub API
        VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases" \
            | grep -o '"tag_name": "v[^"]*"' | head -2 | tail -1 | cut -d'"' -f4)
    fi
    VERSION="${VERSION#v}"
fi

[[ -n "$VERSION" ]] || die "Could not determine rollback version (no git tags / no releases)."

info "Rollback target: v${VERSION}"

# Download & verify
os=$(uname -s | tr '[:upper:]' '[:lower:]')
arch=$(uname -m)
case "$arch" in
    x86_64|amd64)   target="x86_64-unknown-linux-gnu" ;;
    aarch64|arm64)  target="aarch64-unknown-linux-gnu" ;;
    armv7l|armhf)   target="armv7-unknown-linux-gnueabihf" ;;
    *) die "Unsupported arch: $arch" ;;
esac

if [[ "$os" = "darwin" ]]; then
    target="${target/unknown-linux-gnu/apple-darwin}"
fi

url="https://github.com/${REPO}/releases/download/v${VERSION}/ramshield-${target}.tar.gz"
tmpdir=$(mktemp -d)
trap "rm -rf ${tmpdir}" EXIT

info "Downloading v${VERSION} (${target})..."
curl -fsSL "$url" -o "${tmpdir}/ramshield.tar.gz" || die "Download failed"

info "Verifying checksum..."
curl -fsSL "${url}.sha256" -o "${tmpdir}/ramshield.tar.gz.sha256" || die "Checksum sidecar missing"
(cd "${tmpdir}" && sha256sum -c ramshield.tar.gz.sha256) || die "Checksum mismatch"

info "Installing..."
tar -xzf "${tmpdir}/ramshield.tar.gz" -C "${tmpdir}"
sudo install -m 755 "${tmpdir}/ramshield" "${PREFIX}/bin/ramshield" || \
    install -m 755 "${tmpdir}/ramshield" "${PREFIX}/bin/ramshield"

# Apply caps if root/sudo available
if [[ $(id -u) -eq 0 ]] || sudo -n true 2>/dev/null; then
    caps='cap_net_admin,cap_perfmon,cap_bpf+eip'
    setcap "${caps}" "${PREFIX}/bin/ramshield" 2>/dev/null \
        || sudo -n setcap "${caps}" "${PREFIX}/bin/ramshield" 2>/dev/null \
        || info "Cap setcap failed; relying on systemd AmbientCapabilities"
fi

# Restart service if systemd unit exists
if systemctl is-active --quiet ramshield.service 2>/dev/null; then
    info "Restarting systemd service..."
    systemctl restart ramshield.service
elif pgrep -x ramshield >/dev/null; then
    info "Restarting manual process..."
    pkill -x ramshield || true
    nohup ramshield --config "${CONFIG_DIR}/config.toml" >/dev/null 2>&1 &
else
    info "No running daemon detected; starting fresh..."
    nohup ramshield --config "${CONFIG_DIR}/config.toml" >/dev/null 2>&1 &
fi

# Poll healthz
info "Polling ${HEALTHZ_URL} (timeout ${POLL_TIMEOUT}s)..."
deadline=$((SECONDS + POLL_TIMEOUT))
while (( SECONDS < deadline )); do
    if curl -fsS --max-time 2 "${HEALTHZ_URL}" | grep -q '"status":"ok"'; then
        info "Healthz OK — rollback verified."
        exit 0
    fi
    sleep 1
done
die "Healthz never returned ok within ${POLL_TIMEOUT}s"