# RamShield Demo

This demonstration proves the entire golden workflow:
```
install → doctor → enable → verify → detect abuse → enforce → explain → restart/recover → expire
```

## Prerequisites

```bash
# One terminal with RamShield
# One terminal with the CLI
# One terminal for test traffic simulation
```

## Step 1: Start a test service

```bash
# Start a simple HTTP server on port 8080
python3 -m http.server 8080 &
```

## Step 2: Start RamShield

```bash
# Validate environment first
ramshield --doctor --config config.dev.toml

# Then start the daemon
cargo run -- --config config.dev.toml --no-xdp
```

## Step 3: Show HEALTHY

```bash
# Verify health
curl http://127.0.0.1:9999/healthz
# Expected: {"status":"ok","reason":"healthy","uptime_secs":...}

# Verify metrics
curl http://127.0.0.1:9999/metrics | grep ramshield_health

# Check IPC server is listening
cargo run --bin ramshield-cli -- stats
```

## Step 4: Run synthetic attack

```bash
# Send high-rate connection reports simulating an attack
# (Replace with actual attack tool when available)
for i in $(seq 1 10); do
  cargo run --bin ramshield-cli -- block "192.168.1.$i" --reason "demo"
done
```

## Step 5: Observe detection

```bash
# Check that blocks were created
cargo run --bin ramshield-cli -- stats
# Expected: blocks_applied > 0
```

## Step 6: Observe enforcement

```bash
# Check each blocked IP
cargo run --bin ramshield-cli -- check "192.168.1.1"
# Expected: blocked: true

# Check a non-blocked IP
cargo run --bin ramshield-cli -- check "10.0.0.1"
# Expected: blocked: false
```

## Step 7: Show incident explanation

```bash
# View the block log
curl http://127.0.0.1:9999/api/history/blocks
```

## Step 8: Kill RamShield

```bash
# Send SIGTERM
kill -TERM $(pgrep ramshield)
# Wait for graceful shutdown message
```

## Step 9: Restart RamShield

```bash
# Start again
cargo run -- --config config.dev.toml --no-xdp
```

## Step 10: Show state recovered

```bash
# Verify blocks survived restart (WAL replay)
cargo run --bin ramshield-cli -- check "192.168.1.1"
# Expected: blocked: true
# Note: In --no-xdp mode with default WAL off, blocks may not persist.
# Enable WAL in config.dev.toml for crash-durable persistence.
```

## Step 11: Wait for expiration

```bash
# Block TTL defaults to 3600s — wait or set --ttl 60 for demo
# After TTL expires:
cargo run --bin ramshield-cli -- check "192.168.1.1"
# Expected: blocked: false
```

## Step 12: Show cleanup

```bash
# Verify no blocked IPs remain
cargo run --bin ramshield-cli -- stats
# Expected: blocked: 0
```

## Complete Automated Demo

```bash
#!/bin/bash
set -euo pipefail

echo "=== RamShield Demo ==="
echo "Step 1: Starting HTTP test service"
python3 -m http.server 8080 &
HTTP_PID=$!
trap "kill $HTTP_PID 2>/dev/null" EXIT

echo "Step 2: Starting RamShield"
cargo run -- --config config.dev.toml --no-xdp &
RAMSHIELD_PID=$!
trap "kill $RAMSHIELD_PID $HTTP_PID 2>/dev/null" EXIT
sleep 2

echo "Step 3: Verify health"
curl -s http://127.0.0.1:9999/healthz | jq .

echo "Step 4: Block test IPs"
for i in 1 2 3; do
  cargo run --bin ramshield-cli -- block "192.168.1.$i" --reason "demo"
done

echo "Step 5: Verify blocks"
cargo run --bin ramshield-cli -- check "192.168.1.1"

echo "Step 6: Kill and restart"
kill -TERM $RAMSHIELD_PID
sleep 2

echo "Step 7: Restart"
cargo run -- --config config.dev.toml --no-xdp &
RAMSHIELD_PID=$!
sleep 2

echo "Step 8: Check recovery"
cargo run --bin ramshield-cli -- check "192.168.1.1"

echo "=== Demo complete ==="
```