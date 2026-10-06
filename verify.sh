#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$SCRIPT_DIR"
BIN="$ROOT_DIR/target/release/ramshield"
CONFIG="${1:-$ROOT_DIR/config.stress.toml}"
DASH_PORT="${DASH_PORT:-9999}"
IPC_PORT="${IPC_PORT:-7890}"

cleanup() {
    echo "Shutting down server..."
    if [[ -n "${SERVER_PID:-}" ]]; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

echo "Building project..."
cargo build --release --locked --features full -q

echo "Running tests..."
cargo test --workspace --locked --features full -q

echo "Starting server on config: $CONFIG"
"$BIN" --config "$CONFIG" &
SERVER_PID=$!

ready=0
for _ in $(seq 1 50); do
    if curl -sf "http://127.0.0.1:${DASH_PORT}/healthz" >/dev/null 2>&1; then
        ready=1
        break
    fi
    sleep 0.1
done
(( ready == 1 )) || { echo "Error: server did not become ready"; exit 1; }

echo "Checking health endpoint (dashboard :${DASH_PORT})..."
curl -sf "http://127.0.0.1:${DASH_PORT}/healthz" | jq -e '.healthy == true' > /dev/null \
    || { echo "Error: Health check failed"; exit 1; }

echo "Checking metrics endpoint..."
metrics="$(curl -sf "http://127.0.0.1:${DASH_PORT}/metrics")"
grep -q '^# TYPE ' <<<"$metrics" \
    || { echo "Error: Metrics check failed"; exit 1; }
grep -q '^ramshield_' <<<"$metrics" \
    || { echo "Error: Metrics payload empty"; exit 1; }

echo "Checking dashboard API..."
curl -sf "http://127.0.0.1:${DASH_PORT}/api/snapshot" | jq -e '.status == "ok" or .detector_mode != null' > /dev/null \
    || { echo "Error: Dashboard API failed"; exit 1; }

echo "All checks passed successfully!"
