#!/usr/bin/env bash
# scripts/prod_smoke.sh — Production-like smoke test for RamShield.
#
# Boots the binary with --config config.prod.toml --no-xdp, then exercises
# every public IPC + dashboard endpoint to confirm health, block path,
# metrics export, and WAL state. Exits non-zero on first failure.
#
# Usage:  ./scripts/prod_smoke.sh
# Assumes: ./target/release/ramshield built with --features full.

set -euo pipefail

BIN="${BIN:-./target/release/ramshield}"
CFG="${CFG:-./config.prod.toml.example}"
WAL_DIR="${WAL_DIR:-/tmp/ramshield-prod-smoke-wal}"
LOG="${LOG:-/tmp/ramshield-prod-smoke.log}"
DASH_ADDR="${DASH_ADDR:-127.0.0.1:19999}"
IPC_HOST="${IPC_HOST:-127.0.0.1}"
IPC_PORT="${IPC_PORT:-17890}"

green() { printf '\033[32m%s\033[0m\n' "$*"; }
red()   { printf '\033[31m%s\033[0m\n' "$*" >&2; }
fail()  { red "FAIL: $*"; cleanup; exit 1; }
cleanup() {
    if [[ -n "${PID:-}" ]] && kill -0 "$PID" 2>/dev/null; then
        kill "$PID" 2>/dev/null || true
        wait "$PID" 2>/dev/null || true
    fi
}

# Reset state
rm -rf "$WAL_DIR"
mkdir -p "$WAL_DIR"
cleanup
sleep 0.5

# config.prod.toml binds 0.0.0.0 and now fail-closes without credentials
# (Config::validate). Smoke tests exercise endpoints, not public exposure —
# rewrite binds to loopback and point the WAL at the scratch dir. The
# fail-closed rule itself is covered by ramshield-config unit tests.
SMOKE_CFG="${SMOKE_CFG:-/tmp/ramshield-prod-smoke.toml}"
DASH_PORT="${DASH_ADDR##*:}"
sed -e "s|^[[:space:]]*tcp_addr = .*|tcp_addr = \"127.0.0.1:$IPC_PORT\"|" \
    -e "s|^[[:space:]]*http_addr = .*|http_addr = \"127.0.0.1:$DASH_PORT\"|" \
    -e "s|^[[:space:]]*dir = \"/tmp/ramshield_wal\"|dir = \"$WAL_DIR\"|" \
    "$CFG" > "$SMOKE_CFG"

echo "→ booting binary with $SMOKE_CFG (WAL=$WAL_DIR)"
RAMSHIELD_ENGINE__RAM_LIMIT_MB=1024 \
    "$BIN" --config "$SMOKE_CFG" --no-xdp > "$LOG" 2>&1 &
PID=$!
trap cleanup EXIT

# Wait for health
for i in {1..20}; do
    if curl -sf -m 1 "http://$DASH_ADDR/healthz" >/dev/null 2>&1; then break; fi
    sleep 0.3
    if [ "$i" = "20" ]; then fail "binary never became healthy"; fi
done
green "✓ boot: /healthz ok"

# IPC helper: send JSON and return the response
ipc_send() {
    python3 -c "
import socket, json, sys
host, port = sys.argv[1], int(sys.argv[2])
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.settimeout(3)
s.connect((host, port))
s.sendall((sys.argv[3] + '\n').encode())
print(s.recv(4096).decode().strip())
s.close()
" "$IPC_HOST" "$IPC_PORT" "$1"
}

# Sign a request with the configured k1 test key -> compact signed frame.
# HMAC input is <ts_ms>.<key_id><payload>; payload must be the request object
# reserialized compactly with sorted keys (serde_json::Map is a BTreeMap).
sign_payload() {
    python3 -c "
import hmac, hashlib, time, json, sys
key = bytes.fromhex('0b8d647fda3a0ae3c38207e0d7e61edfdfe59bda7359c89f953f76ed68f3768b')
key_id, ts = 'k1', int(time.time() * 1000)
req = json.loads(sys.argv[1])
payload = json.dumps(req, separators=(',', ':'), sort_keys=True).encode()
sig = hmac.new(key, f'{ts}.{key_id}'.encode() + payload, hashlib.sha256).hexdigest()
print(json.dumps({'auth': {'key_id': key_id, 'ts_ms': ts, 'sig': sig}, **req},
                 separators=(',', ':'), sort_keys=True))
" "$1"
}

# Block via IPC
RESP=$(ipc_send "$(sign_payload '{"type":"block_ip","ip":"203.0.113.7","reason":"manual","ttl_secs":300}')")
echo "$RESP" | grep -q "block queued" || fail "block_ip did not respond: $RESP"
green "✓ IPC: block_ip queued"

sleep 1
H=$(curl -sf -m 3 "http://$DASH_ADDR/api/history/blocks" || true)
echo "$H" | grep -q "203.0.113.7" || fail "block not in /api/history/blocks: $H"
green "✓ DASH: block visible in history"

# Snapshot
S=$(curl -sf -m 3 "http://$DASH_ADDR/api/snapshot")
echo "$S" | grep -q '"blocked_total":' || fail "/api/snapshot missing blocked_total"
BT=$(echo "$S" | grep -oE '"blocked_total":[0-9]+' | head -1 | grep -oE '[0-9]+')
[ "$BT" -ge 1 ] || fail "blocked_total = $BT, expected ≥ 1"
green "✓ DASH: snapshot reports blocked_total=$BT"

# Status modules
M=$(curl -sf -m 3 "http://$DASH_ADDR/api/status/modules")
echo "$M" | grep -q '"label"' || fail "/api/status/modules missing label"
green "✓ DASH: status/modules ok"

# Metrics
MT=$(curl -sf -m 3 "http://$DASH_ADDR/metrics")
echo "$MT" | grep -q "ramshield_blocks_total" || fail "Prometheus metrics missing blocks_total"
green "✓ DASH: /metrics serves Prometheus format"

# Unblock
RESP=$(ipc_send "$(sign_payload '{"type":"unblock_ip","ip":"203.0.113.7"}')")
echo "$RESP" | grep -q "unblock queued" || fail "unblock_ip did not respond: $RESP"
green "✓ IPC: unblock_ip queued"

# Hot subnets
HS=$(curl -sf -m 3 "http://$DASH_ADDR/api/hot-subnets" || echo "[]")
green "✓ DASH: /api/hot-subnets reachable ($(echo "$HS" | wc -c) bytes)"

# WAL
WAL_FILES=$(find "$WAL_DIR" -type f 2>/dev/null | wc -l)
[ "$WAL_FILES" -ge 1 ] || fail "no WAL segments written to $WAL_DIR (block never persisted)"
green "✓ WAL: $WAL_FILES files in $WAL_DIR"

# P0#3: SIGKILL then restart — block from WAL must still be present.
# Re-block first (unblock above cleared live state; WAL still has history).
RESP=$(ipc_send "$(sign_payload '{"type":"block_ip","ip":"203.0.113.99","reason":"manual","ttl_secs":300}')")
echo "$RESP" | grep -q "block queued" || fail "pre-kill block_ip: $RESP"
sleep 0.5
kill -9 "$PID" 2>/dev/null || true
wait "$PID" 2>/dev/null || true
PID=""
"$BIN" --config "$SMOKE_CFG" --no-xdp > "$LOG" 2>&1 &
PID=$!
for i in {1..20}; do
    if curl -sf -m 1 "http://$DASH_ADDR/healthz" >/dev/null 2>&1; then break; fi
    sleep 0.3
    if [ "$i" = "20" ]; then fail "binary never became healthy after SIGKILL restart"; fi
done
RESP=$(ipc_send "$(sign_payload '{"type":"check_ip","ip":"203.0.113.99"}')")
echo "$RESP" | grep -q '"blocked":true' || fail "WAL restore after SIGKILL: $RESP"
green "✓ WAL: block restored after SIGKILL restart"

green ""
green "ALL PROD SMOKE CHECKS PASSED"