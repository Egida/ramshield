# RamShield

Kernel-assisted IP/CIDR enforcement with bounded-memory traffic detection, a local control API, WAL-backed state, and an operator dashboard.

Status: controlled single-node pilot. The production-readiness review still lists external-exposure, supervision, capacity, OCI, and operational gaps. Do not treat this repository as a turnkey internet-facing deployment.

[![Rust](https://img.shields.io/badge/Rust-2024-orange?logo=rust)](https://www.rust-lang.org/)
[![CI](https://img.shields.io/badge/CI-review%20pipeline-blue?logo=githubactions)](https://github.com/grep999/ramshield/actions)
[![License](https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue)](LICENSE)

## What it does

RamShield separates telemetry ingestion, detection, enforcement, and observation:

- Ingests single or batched connection reports over a JSON/TCP IPC protocol.
- Tracks IPv4 `/24` and IPv6 `/64` subnet activity with bounded structures.
- Detects per-IP and subnet anomalies using EWMA/forecasting, rate windows, pulse-wave detection, and dual-gate swarm detection.
- Applies temporary IP blocks and CIDR blocks.
- Loads IPv4/IPv6 block prefixes into the eBPF/XDP dataplane when XDP is enabled.
- Persists enforcement state in a compressed, checksummed WAL and replays it at startup.
- Exposes health, metrics, snapshots, history, active blocks, subnet traffic, module status, configuration, and an SSE stream.
- Serves a browser dashboard through Axum.
- Keeps XDP optional: the daemon can run in no-XDP/degraded mode for local integration and development.

## Current release facts

| Item | Current value |
|---|---|
| Workspace version | `0.2.0` |
| Rust edition | 2024 |
| Workspace crates | 13 |
| Rust tests listed | 257 (244 passed, 13 ignored) |
| Final integration suite | 48/48 passed (`scripts/final_integration.py`) |
| Review pipeline | Passed: format, check, Clippy `-D warnings`, tests, metric validation |
| WAL restart test | Passed: one live IP block restored after SIGKILL/restart |
| CIDR XDP verification | 919,408/919,408 packets dropped for `203.0.113.0/24` |
| CIDR test rate | 114,926 packets/s in the isolated netns test |
| Production-like smoke | Passed on isolated ports; live daemon left untouched |

The packet figure is an isolated verification result, not a universal throughput guarantee. Hardware, driver, XDP mode, kernel, packet size, and configuration change capacity.

## Architecture

```text
reverse proxy / client telemetry
              |
              v
       JSON/TCP IPC server
              |
              v
   bounded ingest + pre-aggregation
              |
              v
 detection: IP, /24, /64, forecast, swarm
              |
       +------+------+
       |             |
       v             v
     WAL       enforcement
                     |
          +----------+----------+
          |                     |
          v                     v
   shared runtime state     eBPF/XDP maps
   and dashboard APIs       IP/CIDR drops
```

XDP is an enforcement boundary, not the detector. Detection decisions originate in userspace, then enforcement updates the kernel maps. Without XDP, userspace blocking and dashboard/API behavior remain available; the kernel drop path does not.

## Enforcement

The IPC request contract is defined in [`crates/ramshield-protocol/src/message.rs`](crates/ramshield-protocol/src/message.rs).

Supported requests include:

```json
{"type":"check_ip","ip":"203.0.113.10"}
{"type":"block_ip","ip":"203.0.113.10","reason":"manual","ttl_secs":300}
{"type":"block_cidr","cidr":"203.0.113.0/24","reason":"manual","ttl_secs":300}
{"type":"unblock_ip","ip":"203.0.113.10"}
{"type":"get_ip_stats","ip":"203.0.113.10"}
{"type":"get_stats"}
{"type":"get_status"}
{"type":"report_connections","events":[{"ip":"203.0.113.10","bytes":512,"status_code":200,"proto_fp":1}]}
{"type":"flush"}
```

`ttl_secs` is optional. Requests reject unknown JSON fields. Batch responses report `accepted` and `rejected`; the pipeline invariant is `accepted + rejected == report_connections.events`.

CIDR blocks use longest-prefix matching in the XDP maps. IPv4 and IPv6 prefixes are handled separately. The isolated test uses `203.0.113.0/24` so it cannot collide with the live test topology.

## Dashboard and metrics

Default production-like listeners:

- IPC: `0.0.0.0:7890`
- Dashboard: `0.0.0.0:9999`

Important HTTP routes:

| Route | Purpose |
|---|---|
| `/healthz` | Health status and XDP state |
| `/metrics` | Prometheus exposition |
| `/api/snapshot` | Current pipeline and resource snapshot |
| `/api/stream` | Server-sent event stream |
| `/api/history/batches` | Batch history |
| `/api/history/blocks` | Block history |
| `/api/blocks/active` | Active blocks |
| `/api/traffic/subnets` | Current subnet rows |
| `/api/status/modules` | Module health/status |
| `/api/config` | Redacted configuration; POST is CSRF-checked |

Public binds require configured dashboard authentication and IPC HMAC keys. Keep both services on loopback or behind a trusted authenticated transport during development.

## Quick start

Requirements: Linux, Rust 1.85+, and a locked dependency tree. XDP additionally requires the host capabilities `CAP_NET_ADMIN`, `CAP_BPF`, and `CAP_PERFMON`.

```bash
git clone https://github.com/grep999/ramshield.git
cd ramshield

# Review gate
scripts/review_pipeline.sh

# Release binary
cargo build --release --locked --features full

# Local/no-XDP run; copy and edit the config first
cp config.prod.toml.example config.prod.toml
./target/release/ramshield --config config.prod.toml
```

`config.prod.toml.example` is deliberately fail-closed for public binds. Set an Argon2 dashboard password hash and an IPC HMAC key before exposing listeners. Never commit the resulting secret-bearing config.

For host-NIC XDP, set `[xdp].enabled = true`, choose the target interface and mode, then run with the required capabilities. Verify `xdp_active` through `/healthz` or `/api/snapshot`.

## Configuration baseline

The tracked production-like template uses:

- 256 engine shards and an 8 GiB RAM budget.
- 5,000 per-IP RPS threshold over a 10-second rate window.
- Subnet dual gate: 50 unique IPv4 hosts and 100 events.
- 300-second IP block TTL and 600-second subnet-burst TTL.
- 50 ms batch window and 100 ms pre-aggregation flush interval.
- WAL enabled with fsync durability and compression.
- XDP disabled by default in the template; enable only after host capability and interface validation.

See [`config.prod.toml.example`](config.prod.toml.example) and [`crates/ramshield-config/src/lib.rs`](crates/ramshield-config/src/lib.rs).

## Verification

```bash
# Full review gate
scripts/review_pipeline.sh

# Workspace tests
cargo test --workspace --locked --features full

# Isolated production-like smoke
CFG=config.prod.toml.example \
IPC_PORT=17890 DASH_ADDR=127.0.0.1:19999 \
WAL_DIR=/tmp/ramshield-release-wal \
bash scripts/prod_smoke.sh

# Final integration suite
python3 scripts/final_integration.py

# Privileged isolated XDP/CIDR test
python3 scripts/xdp_netns_sim.py --cidr 203.0.113.0/24
```

Release procedure: [`docs/PRODUCTION_RELEASE_PROCESS.md`](docs/PRODUCTION_RELEASE_PROCESS.md). Readiness ledger: [`docs/PRODUCTION_READINESS.md`](docs/PRODUCTION_READINESS.md).

## Known boundaries

The current readiness review does not claim:

- authenticated external control without deployment-specific TLS/trusted-proxy setup;
- systemd or orchestration supervision and restart policy;
- periodic enforcement reconciliation after runtime map loss;
- a documented capacity envelope or latency SLO;
- a verified immutable OCI digest, signed artifact, SBOM, or rollback image;
- general production readiness for unattended public deployment.

These are release requirements, not hidden features. Keep them visible when promoting a build.

## Repository layout

```text
crates/                  workspace libraries and protocol types
src/                     daemon, engine, IPC, dashboard, enforcement
crates/ramshield-xdp/    userspace XDP loader and eBPF program
scripts/                 review, smoke, integration, audit, and netns tests
docs/                    readiness, release, metrics, and operational records
Containerfile            OCI build definition
```

## Contributing and security

Read [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`DOC_STANDARD.md`](DOC_STANDARD.md) before changing the project. Report vulnerabilities through [`SECURITY.md`](SECURITY.md), not public issues.

License: MIT.
