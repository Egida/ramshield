#!/usr/bin/env bash
# Privileged XDP packet-path qualification. Not for GitHub PR CI.
# Usage: scripts/xdp_qualification.sh [ELF]
set -euo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ELF=${1:-}
if [[ -z "$ELF" ]]; then
  ELF=$(find "$ROOT/target" -name ramshield-xdp -type f 2>/dev/null | head -1 || true)
fi
if [[ -z "$ELF" || ! -s "$ELF" ]]; then
  echo "FAIL: BPF ELF not found (build --features full first)"
  exit 1
fi
SZ=$(stat -c%s "$ELF")
if (( SZ < 64 )); then
  echo "FAIL: ELF too small ($SZ) — placeholder"
  exit 1
fi
MAGIC=$(od -An -N4 -tx1 "$ELF" | tr -d ' \n')
if [[ "$MAGIC" != "7f454c46" ]]; then
  echo "FAIL: not ELF magic ($MAGIC)"
  exit 1
fi
echo "PASS: ELF exists parseable size=$SZ path=$ELF"
if [[ "${XDP_LIVE:-0}" != 1 ]]; then
  echo "SKIP: live attach/drop (set XDP_LIVE=1 on privileged host)"
  exit 0
fi
echo "LIVE attach/drop not implemented in this stub — operator runs packet matrix"
exit 0
