# Deployment Commands Reference

Core operational commands for installing and running RamShield as a security appliance.

## Build

```bash
cargo build --release --locked --features full
```

For development builds faster:
```bash
cargo build --locked --features full
```

## Configuration

### Production config
```bash
# Copy baseline and secure
cp config.baseline.toml config.toml

# Set secrets via environment (recommended)
export RAMSHIELD_DASHBOARD__ADMIN_PASSWORD_HASH='<argon2-hash>'
export RAMSHIELD_IPC__AUTH_KEYS='key1,key2'

# Or edit config.toml directly (never commit secrets)
vim config.toml
```

### Environment override examples
```bash
# Detection tuning
export RAMSHIELD_DETECTION__RPS_THRESHOLD=1000
export RAMSHIELD_DETECTION__BATCH_WINDOW_MS=25

# Engine limits
export RAMSHIELD_ENGINE__RAM_LIMIT_MB=512

# IPC bind address (non-loopback)
export RAMSHIELD_IPC__TCP_ADDR='0.0.0.0:7890'
export RAMSHIELD_TRUSTED_PROXIES='10.0.0.0/8'

# Dashboard
export RAMSHIELD_DASHBOARD__HTTP_ADDR='0.0.0.0:9999'
```

## Initial Setup

### No XDP (development / container)
```bash
RAMSHIELD_AUTONOMOUS__ENABLED=false \
RAMSHIELD_NATIVE_INGEST__ENABLED=false \
RAMSHIELD_SYNPROXY__ENABLED=false \
./target/release/ramshield --config config.toml --no-xdp
```

### Production with XDP
```bash
./target/release/ramshield --config config.toml
```

XDP attach failure blocks startup by default. For degraded mode:
```toml
[xdp]
allow_inband_fallback = true
```

## Health and Verification

```bash
# Health check
curl -fsS http://127.0.0.1:9999/healthz

# Prometheus metrics
curl -fsS http://127.0.0.1:9999/metrics

# Live status
./target/release/ramshield-cli stats

# Per-IP info
./target/release/ramshield-cli info 203.0.113.7
```

## CLI Command Reference

### Query
```bash
# Check if IP is blocked
ramshield-cli check 203.0.113.7
```

### Enforcement
```bash
# Block IP for 5 minutes
ramshield-cli block 203.0.113.7 --reason manual --ttl 300

# Unblock
ramshield-cli unblock 203.0.113.7

# Block CIDR prefix
ramshield-cli block-cidr 10.0.0.0/24 --reason attack --ttl 3600
```

### Status
```bash
# System status
ramshield-cli status

# Stats overview
ramshield-cli stats

# Block log
ramshield-cli blocks
```

## WAL and Recovery

```bash
# WAL directory location
WAL_DIR=/var/lib/ramshield/wal

# Verify WAL integrity manually (advanced)
ls -la $WAL_DIR/
```

For production: WAL **must** be enabled with durable sync. The baseline uses `GroupCommit` durability.

## Systemd Service Template

```ini
[Unit]
Description=RamShield DDoS/Detection Service
After=network.target

[Service]
Type=simple
User=ramshield
Group=ramshield
WorkingDirectory=/opt/ramshield
ExecStart=/opt/ramshield/target/release/ramshield --config /opt/ramshield/config.toml
Restart=always
RestartSec=5

# Security hardening
ProtectSystem=strict
ProtectHome=yes
NoNewPrivileges=yes
MemoryDenyWriteExecute=yes

# XDP capability needed (if XDP enabled)
CapabilityBoundingSet=CAP_NET_ADMIN CAP_SYS_ADMIN

[Install]
WantedBy=multi-user.target
```

## Log Rotation

Standard systemd journal. View:
```bash
journalctl -u ramshield -f
```

## TLS Termination (Dashboard/IPC)

RamShield itself does not speak TLS. Place behind:
- Cloud Load Balancer with HTTPS termination
- Terminator-proxy (Envoy, Caddy, nginx)
- Kubernetes Ingress with TLS cert

Configure `trusted_proxies` and `tls_enabled=true` in config.toml for proper header forwarding and cookie security.

## Uninstall / Stop

```bash
systemctl stop ramshield
systemctl disable ramshield
rm -rf /opt/ramshield  # or wherever installed
```