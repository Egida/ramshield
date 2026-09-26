# RamShield

Kernel-assisted DDoS defense for Linux. RamShield uses proxy telemetry to detect abusive IPs/subnets and, when XDP is active, enforce blocks in the kernel before traffic reaches the application.

## Current status

**Controlled single-node pilot. Not a turnkey internet-facing product.**

Current source version: `0.2.0`. Unreleased work is tracked in `CHANGELOG.md`.

## What it does

- Receives proxy telemetry over JSON/TCP IPC.
- Detects abusive traffic with bounded-memory rate/anomaly logic.
- Persists block decisions with the optional WAL.
- Applies IP/CIDR blocks through eBPF/XDP when the XDP path is active.
- Exposes a local dashboard, health endpoint, and Prometheus metrics.
- Provides a small `ramshield-cli` for status, inspection, block and unblock operations.

When XDP is disabled or unavailable, RamShield can continue with in-band enforcement, but packets are not dropped at the kernel dataplane.

## What it does not do

- It does not protect traffic that never reaches the telemetry source.
- It does not provide upstream or carrier-level DDoS scrubbing.
- It is single-node; there is no verified replicated protection state.
- The dashboard and IPC do not provide built-in transport encryption. Use loopback or put them behind the appropriate trusted/TLS boundary.
- CGNAT handling is experimental and can produce false positives.

## Protection path

```text
client / proxy
      |
      | telemetry
      v
 JSON/TCP IPC
      |
      v
 bounded ingest + detection
      |
      v
 block decision
      |
      +------> WAL (when enabled)
      |
      v
 enforcement
      |
      v
 eBPF/XDP
      |
      v
 Nginx / HAProxy / application
```

## Quick start

Build from source:

```bash
git clone https://github.com/grep999/ramshield.git
cd ramshield
cargo build --release --locked --features full
```

For a local run without XDP:

```bash
cp config.baseline.toml config.toml
./target/release/ramshield --config config.toml --no-xdp
```

Check the daemon:

```bash
curl http://127.0.0.1:9999/healthz
./target/release/ramshield-cli status
```

See [Quickstart](docs/QUICKSTART.md) for the complete local path.

## Documentation

- [Quickstart](docs/QUICKSTART.md)
- [Operations](docs/OPERATIONS.md)
- [Configuration](docs/CONFIGURATION.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Development](docs/DEVELOPMENT.md)

## Before using XDP

XDP is Linux/kernel/NIC dependent and requires the capabilities documented in [Quickstart](docs/QUICKSTART.md). Treat XDP as the enforcement boundary: `xdp_active=true` means the kernel dataplane is attached; a running daemon alone does not prove kernel enforcement.

## License

MIT