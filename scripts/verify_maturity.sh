#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

pass(){ printf '[PASS] %s\n' "$*"; }
warn(){ printf '[WARN] %s\n' "$*"; }
fail(){ printf '[FAIL] %s\n' "$*" >&2; exit 1; }
need(){ command -v "$1" >/dev/null 2>&1 || fail "missing required tool: $1"; }

need cargo
cargo fmt --all -- --check
pass 'cargo fmt'
cargo check --workspace --all-targets --all-features
pass 'cargo check'
cargo test --workspace --all-targets --all-features
pass 'cargo test'
cargo clippy --workspace --all-targets --all-features -- -D warnings
pass 'cargo clippy'

if command -v nft >/dev/null 2>&1; then
  nft -c -f tests/fixtures/synproxy.nft
  pass 'nft SYNPROXY fixture syntax'
else
  warn 'nft not installed; skipped nft syntax gate'
fi

if command -v clang >/dev/null 2>&1; then
  clang -std=c11 -fsyntax-only -Wall -Wextra -Werror crates/ramshield-cgnat/include/ramshield_shm.h
  pass 'CGNAT SHM header'
else
  warn 'clang not installed; skipped CGNAT C header gate'
fi

pass 'maturity gates complete; live XDP/NIC/traffic qualification remains required'
