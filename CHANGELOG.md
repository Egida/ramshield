# Changelog

Notable user-facing changes are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/) and releases use Semantic Versioning.

## [Unreleased]

### Added
- Authenticated IPC with HMAC-SHA256 frame auth, key identity, role-based authorization (Telemetry < ReadOnly < Operator < Admin), and replay protection.
- `--no-xdp` daemon flag for local/CI runs without XDP.
- Argon2-protected dashboard sessions and Prometheus `/metrics` endpoint.
- Crash-durable WAL: configurable retention, compression, fsync; replay restores live blocks before XDP reconcile; TTL-expired blocks not resurrected.
- Configurable dashboard block history size (was hardcoded 40 → `[dashboard] block_log_size`).
- XDP enforcement (AyaAyaXdpApplier wired into boot), drop attribution per-IP audit counts, zero-drop gauge.
- Emergency fast-path detection: burst-block IPs before flush.
- Bloom filter saturation self-heal for detection pipeline.
- SPOT-lite empirical extreme-quantile alarm (forecasting P2).
- Bayesian Hypothesis Framework — unified anomaly detector (EWMA variance + CUSUM).
- v6 /64 swarm gate via subnet_index cardinality; family-complete CIDR records.
- SIGINT/SIGTERM/SIGHUP trap for graceful XDP-unbind shutdown.
- Systemd unit and K8s DaemonSet deployment manifests.
- Hot-path benchmarks (subnet_rotation mode: 30 unique /24s per 15s).
- Production smoke test script.
- `ramshield doctor` subcommand (kernel/config/WAL/XDP/capability checks).
- WAL retention_max_bytes: oldest segments pruned on open/rotate.
- Total RAM in dashboard snapshot (real RSS, not harcoded 32GB ref).

### Changed
- Subnet blocking uses distinct source IPs as one gate (reduces whole-subnet reactions to a single burst) with shorter TTL than per-IP blocks.
- Protocol requests reject unknown fields instead of silent acceptance.
- Detection-first prod config: 25ms decision quantum, 250ms staleness backstop.
- Dashboard redesigned: KPI cards, pipeline flow strip, system gauges, SSE live stream, tabs, sticky header.
- Dead legacy modules/codecs removed (2.1K LOC).
- `src/` restructured: unified Store, DetectionEngine, enforcement actor with WAL-first ordering.
- Config: apply_env_overrides extracted; no-config mode honors env.
- CLI: positional config path honored, unknown flags fatal.

### Fixed
- Lock-poisoning hardened across storage paths.
- Oversize IPC connections receive typed error before close.
- IPC auth silent downgrade: reject keyless keys and answer the client.
- XDP surface apply failures: CIDR LPM trie full stops subnet mitigation (was silent).
- Detection export dropped enforcement-block telemetry.
- Storage: Store::insert is shard-locked commit; preserve subnet window baseline on clock rollback.
- CGNAT seqlock fences — payload cannot overtake odd marker.
- Check_ip consults active CIDR blocks, not just per-IP state.
- One subnet decision, one owner (no split-brain detection).
- WAL replay idempotency qualification (replay twice → same state).
- XDP reconcile qualification against userspace truth.
- Enfocement queue full → 503 response.
- Bloom caches promoted IPs, becomes observable.
- Config: invalid env override values fail startup loudly.
- Dashboard panels stay live; SSE stream recovery.
- Metrics: events_rejected_total split into clean per-class counters.
- Unwrap/expect eliminated from production modules (CI no-unwrap gate enforces).

### Performance
- Detection: multi-threaded batch processing via crossbeam channel; bloom insert via shared atomic words (kills per-flush clone); hoist dual-gate read per subnet per flush; defer bloom probe to genuinely cold IPs only; single-lookup emergency crossing check.
- Storage: subnet_index inner sets as plain HashSet (not sharded DashMap).
- IPC: BytesMut split_to replaces Vec::drain — O(1) framing, eliminates quadratic pipelining.
- Enforcement: expirations Vec→HashMap — O(1) TTL dedup replaces O(N) retain sweeps.
- WAL: retention scan only on segment rotation; 100ms fsync cap prevents thundering herd.
- Forecasting: lock-free — remove Mutex from TrafficCounters.
- Engine: multi_thread fixes IPC starvation under attack load (7× 5s timeouts→0, 75k→489k events/phase); event channel 2M→256k saves 200MB RSS.

### Documentation
- Restructured to 6-core docs set: QUICKSTART, OPERATIONS, CONFIGURATION, ARCHITECTURE, DEVELOPMENT; all content folded in from old PRODUCT/FEATURES/PRODUCTION_READINESS/ROADMAP/CAPACITY/DEPLOY/TUNING/UPGRADING files.
- README rewritten as compact GitHub landing page.
- AGENTS.md merged into DEVELOPMENT.md as coding standards.

### Security
- Key-bearing configs no longer tracked; IPC auth key rotated.
- CIDR prefix validation on deserialization.
- metrics/forecasting/config single copies eliminates stale crate divergence.

## [0.2.0] - 2026-07-31

### Added

- Initial project release notes and contributor guidance.

### Changed

- Build and verification guidance was tightened.

### Fixed

- Removed dead imports and unused constants that blocked verification.

[Unreleased]: https://github.com/grep999/ramshield/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/grep999/ramshield/releases/tag/v0.2.0