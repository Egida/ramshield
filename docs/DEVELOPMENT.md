# Development

Building, testing, contributing, releasing, and roadmap.

## Requirements

- Rust nightly toolchain (pinned in `rust-toolchain.toml`)
- Linux kernel ≥ 5.10 (for XDP/eBPF)
- Python 3.8+ (attack simulators, integration scripts)
- `aya` toolchain for eBPF builds (automatic via `build-std`)

## Build

```bash
cargo build --release --locked --features full
```

Features: `full` (tokio + dashboard + XDP), or default (tokio, no dashboard, no XDP).

## Run locally

```bash
cp config.baseline.toml config.toml
./target/release/ramshield --config config.toml --no-xdp
```

## Run tests

```bash
# Full review gate
scripts/review_pipeline.sh

# Workspace tests
cargo test --workspace --locked --features full

# Production smoke test
CFG=config.baseline.toml IPC_PORT=7890 DASH_ADDR=127.0.0.1:19999 \
  WAL_DIR=/tmp/ramshield-wal bash scripts/prod_smoke.sh

# Integration suite
python3 scripts/final_integration.py
```

## Run formatting/linting

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features full -- -D warnings
cargo check --workspace --locked --all-targets --features full
```

## Workspace structure

```text
src/                daemon, engine, CLI, dashboard, IPC
crates/
  ramshield-config/       TOML configuration, validation
  ramshield-detection/    EWMA, Holt-Winters, pulse-wave, dual-gate
  ramshield-enforcement/  block lifecycle, WAL persistence
  ramshield-forecasting/  Holt-Winters + SPOT-lite
  ramshield-metrics/      counters, Prometheus export
  ramshield-protocol/     IPC wire contract
  ramshield-storage/      sharded store, TTL, subnet index
  ramshield-types/        shared types, CIDR validation
  ramshield-xdp/          XDP loader + eBPF program
  ramshield-cgnat/        shared-memory CGNAT telemetry
  ramshield-analytics/    batch analytics
  ramshield-mesh/         multi-instance coordination
scripts/            review, smoke, integration, audit, netns
docs/               documentation
```

## Adding/changing components

1. Map the relation: trace writer → metric → snapshot → route/SSE → dashboard.
2. Update metric keystore (`docs/metrics/metric-keystore.json`) if adding metrics.
3. Make the smallest change. One finding, one commit.
4. Verify: `cargo check` → `cargo clippy` → `cargo test` → runtime check.
5. Update relevant docs (OPERATIONS, CONFIGURATION, TROUBLESHOOTING).

## Working with XDP

- XDP builds require nightly Rust and `build-std=core,alloc` (set in `rust-toolchain.toml`).
- After rebuild: re-apply file capabilities (`setcap`).
- Test XDP changes in a netns: `python3 scripts/xdp_netns_sim.py --cidr 203.0.113.0/24`.
- XDP reconciliation runs periodically — add tests for both per-IP and CIDR paths.

## Release process

See [`docs/PRODUCTION_RELEASE_PROCESS.md`](PRODUCTION_RELEASE_PROCESS.md) for the full procedure.

In brief:

1. Freeze scope. One feature per commit.
2. Run full review gate.
3. Build release binary: `cargo build --release --locked --features full`.
4. Record: commit, version, Cargo.lock hash, config profile, artifact digest.
5. Run production smoke test and integration suite.
6. Tag and publish.

### Versioning

Current: `0.MINOR.PATCH` (pre-1.0). Breaking changes during 0.x are documented in CHANGELOG.

## Roadmap

### 0.3 — Fuzz & hardening (target: Q4 2026)

- [ ] oss-fuzz integration
- [ ] Protocol fuzz coverage ≥ 90%
- [ ] Crash-free after 10M iterations
- [ ] Third-party security audit
- [ ] Remove remaining production `.unwrap()`/`.expect()`

### 1.0 — General Availability

- Zero production unwraps
- Shared-backend WAL (PostgreSQL/S3)
- Config hot-reload
- Documented capacity envelope
- Signed/immutable OCI artifacts

## Debugging

- Run with `RUST_LOG=ramshield=debug` for verbose output.
- Use `scripts/trace_logged.sh` for trace-level capture.
- Check `docs/metrics/metric-keystore.json` for metric ownership.

## Documentation standards

- Every `docs/` page has: title, metadata (audience + date + verification gate), numbered sections.
- Tables for reference data only. Prose for invariants.
- File paths, config keys, metric names: verbatim from source.
- No marketing language.
- Documentation changes go through same PR review as code.
