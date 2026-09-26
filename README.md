# RamShield

Kernel-assisted DDoS defense that detects flooding IPs and subnets in bounded memory and drops traffic at the eBPF/XDP dataplane.

## What it does

Detects malicious traffic from proxy telemetry and drops attacker packets at the kernel level before they reach your application.

## How it works

Ingests telemetry via authenticated JSON/TCP IPC, detects abuse with bounded-memory detection (EWMA, Holt-Winters, pulse-wave), enforces via eBPF/XDP kernel maps.

## Current capabilities

- Bounded-memory detection (8 GiB budget, sharded pre-aggregation)
- eBPF/XDP dataplane for kernel-level packet drops
- WAL-backed enforcement with crash-safe recovery
- HMAC-SHA256 authenticated IPC with role-based authorization
- Argon2-protected dashboard with session cookies and CSRF protection
- Forecast-driven detection (EWMA α=0.3, Holt-Winters β=γ=0.1, SPOT-lite extreme-quantile alarms, CUSUM with debounce)

## Current limitations

- No encryption on IPC or dashboard transport (HMAC authenticates but does not encrypt)
- Single-node only (no multi-node consensus or state replication)
- No Kubernetes/container-native integration (requires `--privileged` or `--cap-add` for BPF/XDP)
- Detection is telemetry-driven (cannot detect attacks that never reach proxy telemetry feed)
- CGNAT detection is experimental (shared egress IPs may cause false positives)

## Quick start

```bash
curl -sL https://github.com/grep999/ramshield/releases/latest/download/ramshield_0.3.0_amd64.deb -o ramshield.deb
sudo dpkg -i ramshield.deb
cp config.baseline.toml config.toml
./target/release/ramshield --config config.toml --no-xdp
```

For XDP on a host NIC, set `[xdp].enabled = true`, pick the interface and mode, and run with the required capabilities:

```bash
sudo setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' target/release/ramshield
```

## Documentation

- Quickstart
- Operations
- Configuration
- Architecture
- Troubleshooting
- Development

## Status

Controlled single-node pilot, not turnkey internet-facing product.

## License

MIT