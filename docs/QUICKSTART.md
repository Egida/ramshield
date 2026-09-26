# Quickstart

Shortest path from zero to verified RamShield operation.

## Requirements

- Linux kernel ≥ 5.15
- Rust nightly toolchain (pinned in `rust-toolchain.toml`)
- For XDP: `CAP_NET_ADMIN`, `CAP_BPF`, `CAP_PERFMON` on the binary

## Install

```bash
git clone https://github.com/grep999/ramshield.git
cd ramshield
cargo build --release --locked --features full
```

## Configure

```bash
cp config.baseline.toml config.toml
```

`config.baseline.toml` binds `127.0.0.1` for IPC and dashboard — safe for loopback local development without authentication.

For external access: set `[dashboard].http_addr`, `[ipc].tcp_addr`, provide `admin_password_hash` and `auth_keys` (via env or overlay).

## Validate configuration

```bash
./target/release/ramshield --config config.toml --doctor
```

Exit 0 = all gates pass.

## Start

```bash
./target/release/ramshield --config config.toml
```

Without XDP (loopback/development):

```bash
./target/release/ramshield --config config.toml --no-xdp
```

For XDP on a host NIC, set `[xdp].enabled = true`, pick interface and mode in config, and set capabilities:

```bash
sudo setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' target/release/ramshield
```

Rebuild drops file capabilities — re-apply after every `cargo build`.

## Check status

```bash
curl http://127.0.0.1:9999/healthz
# → {"status":"ok","xdp_active":false,...}

curl http://127.0.0.1:9999/api/snapshot

# CLI:
./target/release/ramshield-cli status
./target/release/ramshield-cli check <ip>
```

## Run the test

```bash
CFG=config.toml IPC_PORT=7890 DASH_ADDR=127.0.0.1:9999 \
  WAL_DIR=/tmp/ramshield-wal bash scripts/prod_smoke.sh
```

## Verify protection

```bash
ramshield-cli status        # shows xdp_active, blocks, health
curl http://127.0.0.1:9999/metrics | grep ramshield_blocks
```

## Next steps

- [Operations](OPERATIONS.md) — day-to-day operation, health, incidents
- [Configuration](CONFIGURATION.md) — full option reference
- [Architecture](ARCHITECTURE.md) — data flow and component design
- [Troubleshooting](TROUBLESHOOTING.md) — symptoms and fixes
- [Development](DEVELOPMENT.md) — building, testing, contributing
