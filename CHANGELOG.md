# Changelog

Notable user-facing changes are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/) and releases use Semantic Versioning.

## [0.3.1] - unreleased

### Fixed
- XDP failures no longer report healthy kernel protection. `allow_inband_fallback` (default `false`) controls whether XDP attach failure blocks startup vs degrades gracefully.
- `/healthz` and `/api/snapshot` expose `protection_state` (starting/protected/degraded/failed/stopping) and `xdp_configured`.
- WAL open/replay/CIDR-replay failures no longer silently degrade to volatile enforcement (`allow_volatile_fallback` default false).
- XDP builds fail when the BPF artifact cannot be produced or validated (no placeholder ELF).
- Documented XDP LRU eviction: userspace reconcile restores evicted rules (`ramshield_xdp_reconcile_successes_total`).
- Replay cache bounded per authentication key (`per_key_cap`) preventing one-key exhaustion of global cache.
- Config validation enforces minimum bounds for `max_line_length` (≥256), `max_password_length` (≥1), `max_login_attempts` (≥1), WAL retention minimums.

### Security
- Explicitly fail-closed XDP startup when `[xdp].enabled = true` and kernel dataplane cannot be attached.
- Argon2 password verification concurrency bounded via `[auth].argon2_parallelism` (default 4, Semaphore-gated).
- `/metrics` endpoint exempted from dashboard auth (Prometheus counters only, no sensitive data).
- K8s DaemonSet: `argon2-hash` secret required (`optional: false`). NetworkPolicy selectors tightened to specific pods/namespaces.

### Qualification
- `docs/QUALIFICATION_0.3.1.md`: enforcement, persistence, failure semantics, deployment sections.
- `scripts/upgrade_qualification.sh`: 0.3.0→0.3.1 upgrade and rollback test.
- `scripts/review_pipeline.sh`: added config-contract and release-metadata gates.
- `scripts/prod_smoke.sh`: documented full coverage matrix (review 31 Batch 16).
- `scripts/xdp_qual_matrix.sh`: static ELF + contract matrix; `--live` attach/detach is host-gated.
- `scripts/cap_lifecycle_check.sh`, `scripts/run_benchmarks.sh`, `scripts/pcap_replay.sh`, `scripts/verify_release.sh`.
- WAL fuzz: `crates/ramshield-storage/tests/fuzz.rs` (arbitrary segment bytes must not panic).

### Release hygiene
- `release_candidate.sh`: removed hash of missing keystore files; release gate works from clean checkout.
- Removed absolute symlink, `__pycache__` dirs, `.pyc` files from tracked source.
- `scripts/install.sh`: fixed stale `ramshield/beta/rs` path.
- Baseline recorded at `docs/qualification/0.3.1-baseline.txt`.

## [0.3.0] - 2026-09-26

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
- Enforcement queue full → 503 response.
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

[0.3.0]: https://github.com/grep999/ramshield/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/grep999/ramshield/releases/tag/v0.2.0