#!/usr/bin/env bash
set -euo pipefail

# Read-only repository preflight.
# This intentionally does not install packages, mutate BPF maps, restart the
# service, or expose secrets. It is suitable for CI and operator diagnostics.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

fail=0
check() {
  local label="$1"; shift
  if "$@" >/dev/null 2>&1; then
    printf '[OK]   %s\n' "$label"
  else
    printf '[FAIL] %s\n' "$label"
    fail=1
  fi
}

check 'workspace Cargo.toml exists' test -f Cargo.toml
check 'Cargo.lock exists' test -f Cargo.lock
check 'Rust toolchain policy exists' test -f rust-toolchain.toml
check 'enforcement source exists' test -f crates/ramshield-enforcement/src/lib.rs
check 'XDP source exists' test -f crates/ramshield-enforcement/src/xdp.rs
check 'config source exists' test -f crates/ramshield-config/src/lib.rs
check 'runbook exists' test -f docs/RAMSHIELD_RUNBOOK.md
check 'XDP inspection script exists' test -x scripts/ramshield-xdp-inspect.sh
check 'WAL inspection script exists' test -x scripts/ramshield-wal-inspect.sh

if command -v cargo >/dev/null 2>&1; then
  printf '[INFO] cargo: %s\n' "$(cargo --version)"
else
  printf '[WARN] cargo is not installed; source-only preflight completed\n'
fi

if command -v bpftool >/dev/null 2>&1; then
  printf '[INFO] bpftool: available\n'
else
  printf '[WARN] bpftool: unavailable; kernel-map inspection will be limited\n'
fi

if grep -R -q 'expected_cidrs' crates/ramshield-enforcement/src/lib.rs crates/ramshield-enforcement/src/xdp.rs; then
  printf '[OK]   CIDR-aware reconciliation contract present\n'
else
  printf '[FAIL] CIDR-aware reconciliation contract missing\n'
  fail=1
fi

if grep -q 'invalid value for {name}' crates/ramshield-config/src/lib.rs; then
  printf '[OK]   invalid env values fail loudly\n'
else
  printf '[FAIL] typed env validation helper missing\n'
  fail=1
fi

exit "$fail"
