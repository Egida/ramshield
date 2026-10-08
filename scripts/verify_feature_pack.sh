#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

pass(){ printf '[PASS] %s\n' "$*"; }
fail(){ printf '[FAIL] %s\n' "$*" >&2; exit 1; }

# Structural feature gates independent of the local toolchain.
grep -q 'PACKET_FANOUT_HASH' src/engine/native.rs || fail 'TPACKET fanout missing'
! grep -q 'PACKET_FANOUT_FLAG_UNIQUEID' src/engine/native.rs || fail 'UNIQUEID fanout regression'
grep -q 'fn ptr_at' crates/ramshield-xdp/ramshield-xdp-bpf/src/main.rs || fail 'XDP ptr_at missing'
grep -q 'fn inc_counter' crates/ramshield-xdp/ramshield-xdp-bpf/src/main.rs || fail 'XDP inc_counter missing'
grep -q 'mod counter' crates/ramshield-xdp/ramshield-xdp-bpf/src/main.rs || fail 'XDP counter module missing'
grep -Fq '(*geneve)' crates/ramshield-xdp/ramshield-xdp-bpf/src/main.rs || fail 'Geneve raw-pointer fix missing'
grep -q 'pub fn snapshot' crates/ramshield-mesh/src/aworset.rs || fail 'mesh snapshot missing'
grep -q 'ssrf_hay' src/engine/waf.rs || fail 'WAF Host-safe SSRF path missing'
! grep -q 'set syn_rate4' src/engine/synproxy.rs || fail 'SYN meter object regression'
grep -q 'nft.*-c' src/engine/synproxy.rs || fail 'SYNPROXY preflight syntax check missing'
grep -q 'hostNetwork: false' deploy/k8s/deployment.yaml || fail 'server Deployment must not use hostNetwork'
grep -q 'hostNetwork: true' deploy/k8s/daemonset.yaml || fail 'node guard must use hostNetwork'
grep -q '0.6.0-node' deploy/k8s/daemonset.yaml || fail 'node guard host-tooling image missing'
grep -q 'enabled = true' deploy/k8s/configmap-node.yaml || fail 'node SYNPROXY is not enabled'
grep -q 'Containerfile.node-guard' deploy/k8s/README.md || fail 'node image contract missing'
grep -q 'ProtectKernelTunables=false' deploy/systemd/ramshield.service || fail 'systemd SYNPROXY tunable contract missing'
grep -q 'L7Cost' crates/ramshield-types/src/error.rs || fail 'L7 cost reason missing'
grep -q 'max_effective_rps' crates/ramshield-detection/src/lib.rs || fail 'weighted L7 cost gate missing'
pass 'feature-pack structural gates'

if command -v cargo >/dev/null 2>&1; then
  cargo fmt --all -- --check
  pass 'cargo fmt'
  cargo check --workspace --all-targets --all-features
  pass 'cargo check'
  cargo test --workspace --all-targets --all-features
  pass 'cargo test'
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  pass 'cargo clippy'
else
  printf '[BLOCKED] cargo is not installed; Rust compile/test gates were not executed.\n' >&2
fi

if command -v nft >/dev/null 2>&1; then
  nft -c -f tests/fixtures/synproxy.nft
  pass 'nft SYNPROXY fixture'
else
  printf '[BLOCKED] nft is not installed; nft runtime syntax gate was not executed.\n' >&2
fi

if command -v clang >/dev/null 2>&1; then
  clang -std=c11 -fsyntax-only -Wall -Wextra -Werror crates/ramshield-cgnat/include/ramshield_shm.h
  pass 'CGNAT SHM C header'
else
  printf '[BLOCKED] clang is not installed; C header gate was not executed.\n' >&2
fi
