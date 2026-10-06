#!/usr/bin/env bash
# Fail-closed upgrade and rollback qualification.
set -euo pipefail
PREFIX="${1:-/tmp/ramshield_upgrade_test}"
OLD_TAG="${2:-v0.3.4}"
NEW_TAG="$(scripts/release_version.sh)"
[[ "$OLD_TAG" != "v$NEW_TAG" ]] || { echo 'old and new release are identical' >&2; exit 1; }
NEW="$PREFIX/new"; OLD="$PREFIX/old"; WAL="$PREFIX/wal"; CFG="$PREFIX/config.toml"
cleanup(){
  status=$?
  if (( status == 0 )); then
    rm -rf "$PREFIX"
  else
    printf 'QUALIFICATION FAILED; preserving evidence at %s\n' "$PREFIX" >&2
  fi
  exit "$status"
}; trap cleanup EXIT
mkdir -p "$NEW" "$OLD" "$WAL"
command -v curl >/dev/null 2>&1 || { echo "curl is required" >&2; exit 1; }
git clone --quiet --branch "$OLD_TAG" --depth 1 https://github.com/grep999/ramshield.git "$OLD"
(cd "$OLD" && cargo build --release --locked --features full)
cargo build --release --locked --features full
cp target/release/ramshield "$NEW/ramshield"
cp target/release/ramshield-cli "$NEW/ramshield-cli" 2>/dev/null || true
cat > "$CFG" <<CFG
[engine]
shard_count=4
worker_threads=1
ram_limit_mb=256
[ipc]
tcp_addr="127.0.0.1:17890"
max_line_length=1048576
auth_keys=["k1:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"]
key_roles=[{key_id="k1", role="Admin"}]
require_auth=true
[dashboard]
enabled=true
http_addr="127.0.0.1:19999"
[wal]
enabled=true
dir="$WAL"
durability="GroupCommit"
compress=false
seg_max_bytes=4194304
retention_max_bytes=33554432
[detection]
rps_threshold=999999
rate_window_secs=10
batch_block_enabled=true
block_ttl_secs=120
bloom_bits=100000
promote_min_events=1000
[xdp]
enabled=false
CFG
wait_ready(){
  local i
  for i in {1..50}; do
    if curl --fail --silent http://127.0.0.1:19999/healthz >/dev/null 2>&1; then return 0; fi
    sleep 0.2
  done
  return 1
}
stop(){ kill "$1" 2>/dev/null || true; wait "$1" 2>/dev/null || true; }
"$OLD/target/release/ramshield" --config "$CFG" >/dev/null 2>&1 &
OLD_PID=$!
wait_ready
CLI_ADDR=(--addr 127.0.0.1:17890)
export RAMSHIELD_IPC_KEY=abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789
"$OLD/target/release/ramshield-cli" "${CLI_ADDR[@]}" block 203.0.113.7 --reason upgrade-test --ttl 120
stop "$OLD_PID"
"$NEW/ramshield" --config "$CFG" >/dev/null 2>&1 &
NEW_PID=$!
wait_ready
"$NEW/ramshield-cli" "${CLI_ADDR[@]}" check 203.0.113.7 | grep -q '"blocked": true'
stop "$NEW_PID"
"$NEW/ramshield" --config "$CFG" >/dev/null 2>&1 &
NEW_PID=$!
wait_ready
"$NEW/ramshield-cli" "${CLI_ADDR[@]}" block 198.51.100.1 --reason rollback-test --ttl 120
stop "$NEW_PID"
"$OLD/target/release/ramshield" --config "$CFG" >/dev/null 2>&1 &
OLD_PID=$!
wait_ready
"$OLD/target/release/ramshield-cli" "${CLI_ADDR[@]}" check 198.51.100.1 | grep -q '"blocked": true'
stop "$OLD_PID"
printf 'PASS: upgrade and rollback preserve enforcement state (%s -> %s)\n' "$OLD_TAG" "v$NEW_TAG"
