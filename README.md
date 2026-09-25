# RamShield

**Kernel-assisted DDoS defense.** Detects flooding IPs and subnets in bounded memory, drops their traffic at the eBPF/XDP dataplane — before it reaches your application.

[![Rust](https://img.shields.io/badge/Rust-2024-orange?logo=rust)](https://www.rust-lang.org/)
[![XDP/eBPF](https://img.shields.io/badge/eBPF-XDP-4f8ef7?logo=linux)](https://prototype-kernel.readthedocs.io/en/latest/bpf/)
[![Version](https://img.shields.io/badge/version-0.2.0-2ea44f)](https://github.com/grep999/ramshield/releases)
[![CI](https://img.shields.io/badge/CI-review%20pipeline-6a737d?logo=githubactions)](https://github.com/grep999/ramshield/actions)
[![Tests](https://img.shields.io/badge/tests-273%20passed%2C%200%20failed-2ea44f)](https://github.com/grep999/ramshield/actions)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

## Table of Contents

- [What Is RamShield?](#what-is-ramshield)
- [Use Cases](#use-cases)
- [Why RamShield?](#why-ramshield)
- [Features](#features)
- [How It Works](#how-it-works)
- [Quick Start](#quick-start)
- [Command Line](#command-line)
- [IPC Protocol](#ipc-protocol)
- [Dashboard & API](#dashboard--api)
- [Configuration](#configuration)
- [Performance](#performance)
- [Testing](#testing)
- [Known Boundaries](#known-boundaries)
- [Repository Layout](#repository-layout)
- [Roadmap](#roadmap)
- [Contributing](#contributing)

## What Is RamShield?

RamShield is a Rust daemon that separates telemetry ingestion, anomaly detection, enforcement, and observation into one bounded pipeline:

1. **Ingest** — connection reports arrive over an authenticated JSON/TCP IPC protocol, single or batched.
2. **Detect** — EWMA rate windows, Holt-Winters forecasting, pulse-wave detection, and dual-gate subnet swarms run in bounded, sharded memory. Stealth profiles (Slowloris, sub-threshold ramps) stay under the tripwire by design.
3. **Enforce** — offending IPs and CIDRs get temporary blocks, pushed into eBPF/XDP maps so the kernel drops packets on the wire.
4. **Observe** — a browser dashboard, Prometheus `/metrics`, an SSE event stream, and a WAL-backed state that survives crashes.

It is a **controlled single-node pilot**, not a turnkey internet-facing product. See [Known Boundaries](#known-boundaries) for what this build does not yet claim.

## Use Cases

| Scenario | How RamShield Helps |
|---|---|
| L7 HTTP flood / DDoS | Per-IP EWMA threshold crossing triggers kernel-level drops in ~108 ms (warm) |
| Distributed botnet swarm | Dual-gate /24 detection (50 unique hosts + 100 events) confirms a swarm; public subnets hard-block once density (>64 hosts) and volume (>50k events in 2 s) both confirm |
| Credential stuffing / API abuse | Configurable per-IP RPS tripwire with automatic temporary blocks (TTL-based) |
| Evasion & pulse-wave attacks | Forecast-driven entropy anomaly detection catches ramp-and-burst profiles |
| Legitimate traffic protection | 0.0000% false-positive rate measured over 200 benign-IP probes (see [Performance](#performance)) |

## Why RamShield?

| Factor | Typical hand-rolled approach | RamShield |
|---|---|---|
| Detection state | `Mutex<HashMap<IP, Counter>>` — lock contention, unbounded growth | 256 ahash shards, capped tracked set with cold-entry eviction |
| Enforcement | Userspace loop parsing logs, then `iptables` | eBPF/XDP maps — kernel drops packets before the socket layer |
| Crash safety | In-memory blocks lost on restart | Compressed, checksummed WAL with fsync/group-commit durability, replayed at boot |
| Observability | Sparse logs | Dashboard, Prometheus metrics, SSE stream, full block/batch history |
| Attack awareness | Fixed threshold or human paging | EWMA + Holt-Winters + entropy anomaly + pulse-wave detection |
| False positives | Frequent — one noisy client blocks a subnet | Subnet blocking keys on distinct source IPs; benign traffic measured at 0 FPR |
| Verification | "Works on my box" | 286-test workspace suite + isolated netns XDP drop verification |

## Features

- **Bounded-memory detection** — 8 GiB budget, sharded pre-aggregator, 0.0004% RAM growth per million events at the 21.3 M-event benchmark.
- **eBPF/XDP dataplane** — IPv4 + IPv6 block prefixes loaded into kernel maps with longest-prefix matching; XDP optional, daemon degrades to userspace blocking.
- **WAL-backed enforcement** — enforcement state survives `SIGKILL`; verified restore of live blocks on restart, shared-backend WAL planned for GA.
- **Authenticated IPC** — HMAC-SHA256 per-frame signing (key rotation-ready `key_id`), clock-skew window ±10 s, `deny_unknown_fields` so typos fail loudly. Mandatory replay protection (bound LRU `ReplayStore` via `verify_authenticated`), role-based authorization (`key_roles`), and transport-bind safety (`behind_tls_proxy`).
- **Argon2-protected dashboard** — admin password hash, session-cookie middleware, CSRF-checked config POST.
- **Forecast-driven detection stack** — EWMA α=0.3, Holt-Winters β=γ=0.1 (seasonality 60 s, z=3.0), SPOT-lite extreme-quantile alarms, CUSUM with debounce.
- **Operator CLI** — zero-dependency `ramshield-cli` binary for check/block/unblock/status.
- **Container-ready** — `Containerfile` for OCI builds.

## How It Works

```text
reverse proxy / client telemetry
              |
              v
    JSON/TCP IPC server  (HMAC-SHA256 authed)
              |
              v
  bounded ingest + sharded pre-aggregation (256 shards)
              |
              v
 detection: EWMA / Holt-Winters / pulse-wave / dual-gate swarm
              |
       +------+------+
       |             |
       v             v
     WAL      enforcement
                   |
        +----------+----------+
        |                     |
        v                     v
 shared runtime state    eBPF/XDP maps
 and dashboard APIs      IP/CIDR drops
```

**XDP is an enforcement boundary, not the detector.** Detection decisions originate in userspace; enforcement then updates the kernel maps. Without XDP, userspace blocking and all dashboard/API behavior remain available — only the kernel drop path is absent.

## Quick Start

Requirements: Linux, a recent **nightly** Rust toolchain ([`rust-toolchain.toml`](rust-toolchain.toml) pins it; `rust-src` is needed for the eBPF build), and a locked dependency tree. The XDP path additionally needs `CAP_NET_ADMIN`, `CAP_BPF`, and `CAP_PERFMON`.

```bash
git clone https://github.com/grep999/ramshield.git
cd ramshield

# Review gate (fmt, clippy -D warnings, full test suite)
scripts/review_pipeline.sh

# Release binary (full feature set: tokio, dashboard, XDP)
cargo build --release --locked --features full

# Run (loopback, no XDP) — copy the config first
cp config.baseline.toml config.toml
./target/release/ramshield --config config.toml
```

`config.baseline.toml` is **loopback-safe by default**: binds `127.0.0.1` for IPC and dashboard, enabling safe local development without authentication. For external access, set `[dashboard].http_addr` and `[ipc].tcp_addr` to your interface, then provide `[dashboard].admin_password_hash` (via `RAMSHIELD_DASHBOARD__ADMIN_PASSWORD` env) and `[ipc].auth_keys` (via `RAMSHIELD_IPC__AUTH_KEYS` env) to avoid startup validation errors.

For host-NIC XDP, set `[xdp].enabled = true`, pick the interface and mode, and run with the required capabilities. Verify `xdp_active` via `/healthz` or `/api/snapshot`.

## Command Line

The daemon ships with a companion CLI ([`src/cli.rs`](src/cli.rs)) that speaks the IPC protocol:

```text
ramshield-cli [--addr 127.0.0.1:7890] [--key <hex>] <COMMAND>

Commands:
  check <ip>              Is this IP currently blocked?
  block <ip> [--reason manual] [--ttl <secs>]
                          Block an IP (TTL defaults to config block_ttl_secs)
  unblock <ip>            Remove an IP block
  unblock-cidr <cidr>     Remove a CIDR block
  stats                   Pipeline counters
  status [--json]         Daemon status (pretty JSON by default)
  info <ip>               Per-IP stats
```

Auth key comes from `RAMSHIELD_IPC_KEY` (hex) or `--key`; when set, frames are HMAC-SHA256-signed. On a server where `auth_keys` is configured, unsigned frames are rejected (`401`). `key_roles` config governs what each key may do (`Telemetry`/`ReadOnly`/`Operator`/`Admin`); a role below the command's requirement returns `403`.

### IPC Protocol

The request contract lives in [`crates/ramshield-protocol/src/message.rs`](crates/ramshield-protocol/src/message.rs). Requests are newline-delimited JSON; unknown fields are rejected. When `[ipc] auth_keys` is non-empty, every frame must carry an `auth` envelope:

```json
{"auth":{"key_id":"k1","ts_ms":1696000000000,"sig":"<hex>"},"type":"check_ip","ip":"1.2.3.4"}
```

The verified `key_id` becomes the enforcement `actor` and drives `key_roles` authorization. Replay of a frame within the ±10 s clock-skew window is rejected (mandatory `ReplayStore`). Full wire contract + signed-frame example: [`docs/IPC.md`](docs/IPC.md).

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

`ttl_secs` is optional. Batch responses report `accepted` and `rejected`; the pipeline invariant is `accepted + rejected == report_connections.events`.

## Dashboard & API

Default listeners: IPC `127.0.0.1:7890`, dashboard `127.0.0.1:9999` (loopback).

| Route | Purpose |
|---|---|
| `/` | Dashboard UI |
| `/login` | Argon2 password login |
| `/healthz` | Health status and XDP state |
| `/metrics` | Prometheus exposition |
| `/api/snapshot` | Current pipeline and resource snapshot |
| `/api/stream` | Server-sent event stream |
| `/api/history/batches` | Batch history |
| `/api/history/blocks` | Block history |
| `/api/blocks/active` | Active blocks |
| `/api/traffic/subnets` | Current subnet rows |
| `/api/status/modules` | Module health/status |
| `/api/config` | Redacted configuration; POST is CSRF-checked and auth-gated |

## Configuration

Config is TOML (`config.toml`, `config.prod.toml` for production-like, `config.debug.toml`, etc.). The tracked canonical template (`config.baseline.toml`) baseline:

| Setting | Value | Meaning |
|---|---|---|
| `engine.shard_count` | 256 | Pre-aggregator shards |
| `engine.ram_limit_mb` | 8192 | Bounded-memory budget |
| `detection.rps_threshold` | 5000 | Per-IP tripwire (RPS) |
| `detection.rate_window_secs` | 10 | Rate window |
| `detection.subnet_batch_threshold` | 50 | Unique source IPs per /24 |
| `detection.subnet_batch_min_events` | 100 | Events/s in the 2 s window |
| `detection.block_ttl_secs` | 300 | Default block lifetime |
| `detection.subnet_burst_ttl_secs` | 600 | Subnet-burst block lifetime |
| `detection.batch_window_ms` | 50 | Batch window |
| `detection.pre_aggs_flush_interval_ms` | 100 | Pre-aggregation flush |
| `wal.durability` | `GroupCommit` | 100 ms window, or `Fsync` for strictest |
| `wal.compress` | `true` | zstd-compressed segments |
| `xdp.enabled` | `false` | Fail-closed default |

Full reference: [`config.baseline.toml`](config.baseline.toml) and [`crates/ramshield-config/src/lib.rs`](crates/ramshield-config/src/lib.rs).

## Performance

Verified results from [`docs/DDOS_BENCHMARK_REPORT.md`](docs/DDOS_BENCHMARK_REPORT.md): 21.3 M events across 21 attack/resilience tests (v2 industry-style + v3 RFC 9411 compliance) on a **single laptop-class host, loopback XDP (generic) mode**. Absolute numbers are machine-class-dependent; the ratios are the portable part.

| Metric | Result | Conditions |
|---|---|---|
| False-positive rate | **0.0000%** (0/200 benign IPs) | BlackNeuron FPR test, T14 |
| Detect → mitigate latency | **108 ms** (warm) | T20; cold start 8 s (one full window) |
| Recovery (unblock → reflect) | **52 ms** | T15, 10 unblocks |
| Raw IPC throughput | **135,602 events/s** | T11, 10 s, 87 transient errors |
| Sustained single-attacker flood | **154,731 events/s** | T2, 30 s |
| Peak burst pattern | **157,031 events/s** | T4, 3×(5 s on/off) |
| Stealth profile (Slowloris) | **0 blocks** on 2.28 M events | T5 — sub-threshold by design |
| RSS footprint | **44 MB flat** across 21.3 M events | §6 memory profile |
| RAM growth | 0.0004% per million events | tracked set capped + cold entry eviction |
| Auto-detected blocks | 137, **0 false positives** among 148 total | §5 detection pipeline |

Known attack-vector gaps from the same report: background throughput degrades ~93.5% under attack (T19) and cold detection needs one full window (8 s). Both are tracked in the readiness ledger, not hidden.

## Testing

```bash
# Full review gate (fmt, clippy -D warnings, tests, metric validation)
scripts/review_pipeline.sh

# Workspace tests — current master: 273 passed, 0 failed, 13 ignored
cargo test --workspace --locked --features full

# Isolated production-like smoke
CFG=config.baseline.toml \
IPC_PORT=17890 DASH_ADDR=127.0.0.1:19999 \
WAL_DIR=/tmp/ramshield-release-wal \
bash scripts/prod_smoke.sh

# Final integration suite
python3 scripts/final_integration.py

# Privileged isolated XDP/CIDR drop verification (netns)
python3 scripts/xdp_netns_sim.py --cidr 203.0.113.0/24
```

Release procedure: [`docs/PRODUCTION_RELEASE_PROCESS.md`](docs/PRODUCTION_RELEASE_PROCESS.md). Readiness ledger: [`docs/PRODUCTION_READINESS.md`](docs/PRODUCTION_READINESS.md). Bench method and raw data: [`docs/DDOS_BENCHMARK_REPORT.md`](docs/DDOS_BENCHMARK_REPORT.md).

## Known Boundaries

The current readiness review does **not** claim:

- authenticated *encrypted* external control without operator TLS / trusted-proxy (`behind_tls_proxy` only asserts the proxy exists; HMAC does not encrypt);
- systemd or orchestration supervision and restart policy;
|- zero-drop enforcement under 32 concurrent enforcement writers (single-writer model);
- a documented capacity envelope or latency SLO;
- a verified immutable OCI digest, signed artifact, SBOM, or rollback image;
- general production readiness for unattended public deployment.

These are release requirements, not hidden features. They are tracked in [`docs/PRODUCTION_READINESS.md`](docs/PRODUCTION_READINESS.md) and the [Roadmap](#roadmap).

## Repository Layout

```text
src/                      daemon, engine, IPC, dashboard, enforcement, CLI
crates/
  ramshield-config/       TOML configuration, validation
  ramshield-detection/    EWMA, forecasting, pulse-wave, dual-gate swarm
  ramshield-enforcement/  block lifecycle, WAL persistence
  ramshield-forecasting/  Holt-Winters + SPOT-lite alarm
  ramshield-metrics/      counters and metric export
  ramshield-protocol/     IPC wire contract
  ramshield-storage/      sharded pre-aggregation, subnet index
  ramshield-types/        shared types and CIDR validation
  ramshield-xdp/          userspace XDP loader + eBPF program (aya)
  ramshield-cgnat/        shared-memory telemetry on the CGNAT path
  ramshield-analytics/    batch analytics
  ramshield-mesh/         multi-instance coordination
scripts/                  review, smoke, integration, audit, netns tests
docs/                     readiness, release, benchmarks, roadmap
Containerfile             OCI build definition
```

## Roadmap

Tracked in [`docs/ROADMAP.md`](docs/ROADMAP.md) — dated milestones, each with a verifiable outcome:

- **0.2 → 0.3 (in progress)** — fuzzing & hardening: protocol fuzz coverage ≥ 90%, crash-free 10 M-iteration runs, third-party security audit, zero production `.unwrap()/.expect()`.
- **1.0 — General Availability** — zero production unwraps, shared-backend WAL (PostgreSQL/S3), config hot-reload, documented capacity envelope, signed/immutable OCI artifacts.

## Contributing

Read [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`DOC_STANDARD.md`](DOC_STANDARD.md) before changing the project. Report vulnerabilities through [`SECURITY.md`](SECURITY.md), not public issues. Changelog: [`CHANGELOG.md`](CHANGELOG.md).

License: [MIT](LICENSE).