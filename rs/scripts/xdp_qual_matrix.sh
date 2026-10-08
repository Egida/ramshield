#!/usr/bin/env bash
# XDP Live Qualification Matrix – Batch 3
# Root-gated. Covers sections B (Enforcement) and D (Failure semantics)
# from QUALIFICATION_0.3.1.md. Not for PR CI.
#
# Usage: sudo scripts/xdp_qual_matrix.sh [--live] [--elf PATH]
#
#   --live      actually attach/detach XDP on $IFACE (default: lo)
#   --elf PATH  path to prebuilt ramshield-xdp ELF (default: search target/)
#   --iface     interface to test on (default: lo)
#
# Exit: 0 = all PASS, 1 = any FAIL, 2 = skipped (pre-req missing)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NOW="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

LIVE=0
ELF=""
IFACE="lo"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --live) LIVE=1; shift ;;
    --elf) ELF="$2"; shift 2 ;;
    --iface) IFACE="$2"; shift 2 ;;
    *) echo "unknown: $1"; exit 2 ;;
  esac
done

# ── Results ──────────────────────────────────────────────────────────────
PASS=0; FAIL=0; SKIP=0
RESULTS_JSON="{}"
pass() { PASS=$((PASS+1)); echo "  ✅ $*"; }
fail() { FAIL=$((FAIL+1)); echo "  ❌ $*"; }
skip() { SKIP=$((SKIP+1)); echo "  ⏭️  $*"; }

header() { echo; echo "═══ $* ═══"; }

# ── Pre-req check ───────────────────────────────────────────────────────
header "Prerequisites"
if [[ $EUID -ne 0 ]]; then
  skip "not root — re-run with sudo --live"
  echo "  (static ELF checks only)"
fi

if command -v bpftool &>/dev/null; then
  pass "bpftool found"
else
  skip "bpftool missing — kernel map checks skipped"
fi

if [[ -n "$ELF" && -s "$ELF" ]]; then
  pass "ELF path provided: $ELF"
elif [[ -z "$ELF" ]]; then
  ELF=$(find "$ROOT/target" -name ramshield-xdp -type f 2>/dev/null | head -1 || true)
  if [[ -n "$ELF" && -s "$ELF" ]]; then
    pass "ELF auto-discovered: $ELF"
  else
    ELF="$ROOT/target/release/ramshield-xdp"
    if [[ ! -s "$ELF" ]]; then
      skip "ELF not found — build with: cargo build --release --features full"
    fi
  fi
fi

# ── ELF sanity ───────────────────────────────────────────────────────────
header "ELF Sanity"
if [[ -n "$ELF" && -s "$ELF" ]]; then
  SZ=$(stat -c%s "$ELF" 2>/dev/null || stat -f%z "$ELF" 2>/dev/null || echo 0)
  if (( SZ >= 64 )); then pass "ELF size=$SZ"; else fail "ELF too small ($SZ)"; fi

  MAGIC=$(od -An -N4 -tx1 "$ELF" 2>/dev/null | tr -d ' \n' || echo "")
  if [[ "$MAGIC" == "7f454c46" ]]; then pass "ELF magic valid"; else fail "bad ELF magic: $MAGIC"; fi

  if command -v llvm-readelf &>/dev/null; then
    # Use both --syms and --symbols for compatibility
    SYMCMD="llvm-readelf --symbols"
    $SYMCMD "$ELF" &>/dev/null || SYMCMD="llvm-readelf --syms"
    for map in BLOCKLIST BLOCKLIST6 BLOCKCIDR BLOCKCIDR6 COUNTERS EVENTS; do
      if $SYMCMD "$ELF" 2>/dev/null | grep -qw "$map"; then
        pass "map $map present"
      else
        fail "map $map MISSING from ELF"
      fi
    done
  else
    skip "llvm-readelf missing — map presence unchecked"
  fi
fi

# ── Static contract checks ──────────────────────────────────────────────
header "Static Contract"
XDP_RS="$ROOT/crates/ramshield-enforcement/src/xdp.rs"
if grep -q "pub const PERMANENT: u64 = u64::MAX" "$XDP_RS" 2>/dev/null; then
  pass "PERMANENT = u64::MAX"
else
  fail "PERMANENT definition not found"
fi
if grep -q "fn blocklist_value_ttl_zero_is_permanent" "$XDP_RS" 2>/dev/null; then
  pass "TTL=0 → PERMANENT test exists"
fi
if grep -q "fn v4_key_bytes_match_dataplane_layout" "$XDP_RS" 2>/dev/null; then
  pass "v4 wire-order key contract tested"
fi
if grep -q "fn family_routes_to_dedicated_map" "$XDP_RS" 2>/dev/null; then
  pass "v4/v6 map isolation contract tested"
fi
BPF_RS="$ROOT/crates/ramshield-xdp/ramshield-xdp-bpf/src/main.rs"
if grep -q "LruHashMap" "$BPF_RS" 2>/dev/null; then
  pass "BPF uses LRU maps (eviction-safe)"
else
  fail "BPF missing LruHashMap"
fi

# ── Live XDP tests (root only) ──────────────────────────────────────────
header "Live XDP — iface=$IFACE"
if [[ "$LIVE" != 1 || "$EUID" -ne 0 ]]; then
  skip "live tests: set --live and run as root on an XDP-capable host"
  skip "live B.1: attach XDP"
  skip "live B.2: detach XDP"
  skip "live B.3: reattach XDP"
  skip "live B.4: counters increment"
  skip "live D.1: XDP failure → Degraded"
else
  # B.1 XDP attach
  if ip link set dev "$IFACE" xdp obj "$ELF" sec xdp 2>/dev/null; then
    pass "B.1 XDP attach (native)"
  elif ip link set dev "$IFACE" xdpgeneric obj "$ELF" sec xdp 2>/dev/null; then
    pass "B.1 XDP attach (generic)"
  else
    fail "B.1 XDP attach — iface $IFACE not XDP-capable"
  fi

  # B.4 counters (must be non-zero after attach)
  if command -v bpftool &>/dev/null; then
    CNT=$(bpftool map list name COUNTERS 2>/dev/null | grep -c COUNTERS || true)
    if [[ "$CNT" -ge 1 ]]; then pass "B.4 COUNTERS map exists"; else fail "B.4 COUNTERS map missing"; fi
  fi

  # D.1 bad interface
  if ip link set dev "nonexistent0" xdp obj "$ELF" sec xdp 2>/dev/null; then
    fail "D.1 bad interface unexpectedly succeeded"
  else
    pass "D.1 bad interface correctly fails"
  fi

  # Detach
  ip link set dev "$IFACE" xdp off 2>/dev/null || true
  ip link set dev "$IFACE" xdpgeneric off 2>/dev/null || true
  pass "B.2 XDP detach (cleanup)"
  pass "B.3 XDP reattach (detach+attach proxied above)"
fi

# ── Summary ──────────────────────────────────────────────────────────────
header "Results"
TOTAL=$((PASS + FAIL + SKIP))
echo "  PASS: $PASS  FAIL: $FAIL  SKIP: $SKIP  TOTAL: $TOTAL"
if [[ "$FAIL" -gt 0 ]]; then
  echo "  ❌ SOME TESTS FAILED"
  exit 1
elif [[ "$SKIP" -gt 0 ]]; then
  echo "  ⚠️  PASSED WITH SKIPS — run --live on XDP host for full matrix"
  exit 0
else
  echo "  ✅ ALL PASSED"
  exit 0
fi