#!/usr/bin/env bash
set -euo pipefail

# Read-only XDP/BPF inspection helper.
# Usage: ./scripts/ramshield-xdp-inspect.sh [interface]
# No map mutation is performed by this script.

IFACE="${1:-}"

if ! command -v bpftool >/dev/null 2>&1; then
  echo "bpftool is required for kernel-map inspection" >&2
  exit 2
fi

if [[ -n "$IFACE" ]] && command -v ip >/dev/null 2>&1; then
  echo "== XDP link: $IFACE =="
  ip -details link show dev "$IFACE" || true
fi

echo "== XDP programs =="
bpftool prog show || true

echo "== BPF maps =="
bpftool map show || true

echo "== RamShield-related maps =="
bpftool map show | grep -E 'BLOCKLIST|BLOCKCIDR|COUNTERS|EVENTS' || true

echo
cat <<'NOTE'
Interpretation:
- BLOCKLIST/BLOCKLIST6 are per-IP maps.
- BLOCKCIDR/BLOCKCIDR6 are CIDR LPM maps.
- A present userspace block with an empty corresponding kernel map is map drift.
- Do not mutate maps from this script; let the enforcement actor reconcile them.
NOTE
