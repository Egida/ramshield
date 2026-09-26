# RamShield

**Linux-native traffic protection with eBPF/XDP.**

RamShield watches traffic from your proxy, spots abusive patterns, and can block offending IPs or networks directly at the kernel level.

```text
                    traffic
                       │
                       ▼
                reverse proxy
                       │
                   telemetry
                       │
                       ▼
                  RamShield
                ┌──────┴──────┐
                │             │
            detection       state
                │             │
                └──────┬──────┘
                       │
                    decision
                       │
                       ▼
                    XDP/eBPF
                       │
                       ▼
                  application
```

The basic loop is simple:

**observe → detect → decide → block → expire**

## Run it

Build from source:

```bash
git clone https://github.com/grep999/ramshield.git
cd ramshield

cargo build --release --locked --features full
```

Start locally:

```bash
cp config.baseline.toml config.toml

./target/release/ramshield \
  --config config.toml \
  --no-xdp
```

Check that it is running:

```bash
curl http://127.0.0.1:9999/healthz
./target/release/ramshield-cli status
```

See the [Quickstart](docs/QUICKSTART.md) for the full setup.

## What happens when traffic turns bad

RamShield collects telemetry, keeps a bounded view of recent activity, and looks for traffic that crosses the configured detection rules.

A block can then move through:

```text
detection
   ↓
decision
   ↓
WAL
   ↓
enforcement
   ↓
XDP
```

Blocks have TTLs, can be inspected from the CLI, and are removed when they expire.

```bash
ramshield-cli status
ramshield-cli stats
ramshield-cli check <ip>
ramshield-cli info <ip>
```

Manual blocks are available too:

```bash
ramshield-cli block <ip> --reason manual --ttl 300
ramshield-cli unblock <ip>
ramshield-cli unblock-cidr <cidr>
```

## XDP

When XDP is enabled, enforcement happens close to the network interface:

```text
packet
  ↓
NIC
  ↓
XDP
  ├── drop
  └── pass
```

RamShield exposes the XDP state so you can see whether the kernel dataplane is actually active.

## Configuration

RamShield uses TOML.

A starting point is included in the repository:

```text
config.baseline.toml
```

The configuration covers detection, batching, IPC, dashboard, WAL, XDP, forecasting and memory limits.

See [Configuration](docs/CONFIGURATION.md).

## Observe it

Health:

```bash
curl http://127.0.0.1:9999/healthz
```

Metrics:

```bash
curl http://127.0.0.1:9999/metrics
```

Dashboard and API are available from the same local service.

## Documentation

[Quickstart](docs/QUICKSTART.md) · [Operations](docs/OPERATIONS.md) · [Configuration](docs/CONFIGURATION.md) · [Architecture](docs/ARCHITECTURE.md) · [Troubleshooting](docs/TROUBLESHOOTING.md) · [Development](docs/DEVELOPMENT.md)

## Development

```bash
cargo test --workspace --locked --features full
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features full -- -D warnings
```

See [Development](docs/DEVELOPMENT.md).

## License

MIT
