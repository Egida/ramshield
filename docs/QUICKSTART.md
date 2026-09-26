# Quickstart

Build and run RamShield locally, then verify that the daemon and CLI can talk to each other.

## Requirements

- Linux for the XDP path.
- The repository-pinned Rust toolchain from `rust-toolchain.toml`.
- `CAP_NET_ADMIN`, `CAP_BPF`, and `CAP_PERFMON` only when using XDP.
- `curl` for the HTTP checks below.

The source tree currently builds the full binary with the `full` feature.

## Build

```bash
git clone https://github.com/grep999/ramshield.git
cd ramshield
cargo build --release --locked --features full
```

## Create a local configuration

Use the repository baseline as the starting template:

```bash
cp config.baseline.toml config.toml
```

For a local development run, disable XDP explicitly:

```bash
./target/release/ramshield --config config.toml --no-xdp
```

The baseline binds IPC and the dashboard to loopback. It also enables WAL and XDP in the file, so `--no-xdp` is useful for an unprivileged local run.

The baseline file is a repository/test configuration, not a universal production tuning profile.

## Start

```bash
./target/release/ramshield --config config.toml --no-xdp
```

The daemon accepts a config path either as `--config <path>` / `-c <path>` or as a single positional path.

## Check health

```bash
curl http://127.0.0.1:9999/healthz
```

A healthy daemon returns HTTP 200 and JSON containing `status`, `reason`, and `uptime_secs`. A non-healthy state returns HTTP 503.

Then check the operator CLI:

```bash
./target/release/ramshield-cli status
```

For JSON:

```bash
./target/release/ramshield-cli status --json
```

## Check a specific IP

```bash
./target/release/ramshield-cli check 203.0.113.7
./target/release/ramshield-cli info 203.0.113.7
```

## Manual block / unblock

```bash
./target/release/ramshield-cli block 203.0.113.7 --reason manual --ttl 300
./target/release/ramshield-cli check 203.0.113.7
./target/release/ramshield-cli unblock 203.0.113.7
```

`--ttl` is in seconds.

## Verify metrics

```bash
curl http://127.0.0.1:9999/metrics
```

The dashboard exposes Prometheus text format.

## Stop

Press `Ctrl+C`. The daemon handles SIGINT, SIGTERM, and SIGHUP through the same shutdown path.

## Using XDP

To use the kernel dataplane:

1. Set `[xdp].enabled = true`.
2. Set the target interface and XDP mode.
3. Build with `--features full`.
4. Apply the required capabilities to the actual binary:

```bash
sudo setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' target/release/ramshield
```

5. Start without `--no-xdp`.
6. Confirm the reported `xdp_active` state.

Reapply file capabilities after every rebuild.

## Next steps

- [Operations](OPERATIONS.md)
- [Configuration](CONFIGURATION.md)
- [Architecture](ARCHITECTURE.md)
- [Troubleshooting](TROUBLESHOOTING.md)
- [Development](DEVELOPMENT.md)