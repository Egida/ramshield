# RamShield Production Readiness Review

Review date: 2026-09-26
Branch: `master`
Reviewed commit: `4d0dc80` (P1–P8 merged)

## Verdict

Status: NOT production-ready — **security boundary (Phase 1) is closed**. Remaining blockers: unsigned release artifacts, WAL restart drill under load, capacity envelope (P9), deploy/rollback (P10).

Current state supports a controlled single-node pilot with HMAC-authenticated IPC, role-based authorization, mandatory replay protection, and fail-closed public bind.

Readiness score: 6/10 (was 4/10 before P1–P8).

## Evidence

Green now:

- Review pipeline passed:
  - metric keystore validation
  - generated JSONL freshness
  - Python syntax
  - rustfmt
  - workspace `cargo check`
  - Clippy with `-D warnings`
  - locked full-feature workspace tests
- `full_metrics_verify.py` passed.
- Live daemon healthy:
  - `/healthz`: `status=ok`
  - `xdp_active=true`
  - `events_ingested=164270`
  - `frames_rejected_total=13`
  - `channel_depth=0`
  - `events_shed=0`
- Live telemetry now distinguishes frames, events, batches, decisions, and blocks.
- Metric ownership is registered in `docs/metrics/metric-keystore.json`.
- Trace runner, log auditor, trace coverage probe, and review pipeline exist.

Not proven:

- OCI image build (`docker/Dockerfile` + `scripts/build_docker.sh`) and systemd unit (`deploy/systemd/ramshield.service`) exist but are unsigned; no digest promotion or rollback artifact drill has been exercised.
- `config-xdp.toml` binds localhost, has no active IPC HMAC key, and has no WAL section.
- IPC HMAC authenticates; it does not encrypt. Public bind without `ipc.behind_tls_proxy = true` now fails `validate()` at startup (P4). Loopback remains the default. TLS termination is still the operator's job.
- Runtime reports `wal_lsn=0`; restart durability is not production-proven.
- Enforcement reconciliation runs every ~10 s (`RECONCILE_EVERY_TICKS=40` × 250 ms) but has no exported drift/age metrics and no live map-loss drill in CI.
- `worker_threads` is informational; processing remains single-batch-thread.
- XDP requires host capabilities and manual capability restoration after builds.
- Current source contains 439 production `.unwrap()`/`.expect()` matches under `src/` and `crates/`.
- Live/XDP verification is not part of CI; the review pipeline currently skips it by design.
- No measured SLOs, alert rules, capacity envelope, or rollback drill are recorded.
- No authenticated external-control deployment has been exercised.

## Blocker classification

### P0 — blocks any external production exposure

1. Control-plane security
   - IPC auth keys: rotated and files deleted (commit 6e92c2c); key-history
     in git history is formally accepted — no rewrite risk worth the cost.
   - Activate and rotate IPC HMAC credentials.
   - Define dashboard authentication deployment path.
   - Keep binds private unless TLS or an authenticated reverse proxy is present.
   - Add a negative test proving unauthenticated control/config access fails.

2. Durable state and recovery
   - Enable WAL in the production profile.
   - Prove write → kill → restart → block restored → XDP reconciled.
   - Define behavior for WAL open/replay failure: fail closed or explicit degraded mode.
   - Add disk-full and corrupt-segment drills.

3. Deployment identity and rollback
   - Build an immutable OCI image or a formally managed host package.
   - Record commit, image digest, config hash, keystore hash, schema version, and capabilities.
   - Provide a tested previous image and rollback command.

### P1 — blocks a reliable production service

4. Supervision and lifecycle
   - systemd unit (`deploy/systemd/ramshield.service`) and k8s manifest (`deploy/k8s/daemonset.yaml`) exist but need operator testing; define readiness/liveness probes, graceful shutdown, restart backoff, resource limits, and log retention.
   - Ensure exactly one process owns IPC and dashboard ports.

5. Enforcement convergence
   - Periodic store → XDP reconciliation runs every ~10 s (`RECONCILE_EVERY_TICKS=40`), but no exported drift/age/failure metrics and no live map-loss drill in CI.
   - Export reconciliation age, failures, and drift count.
   - Test XDP map loss/reload while the daemon remains alive.

6. Failure handling
   - Remove or justify the 439 production `.unwrap()`/`.expect()` matches.
   - Convert startup-critical failures into typed, observable exits.
   - Add panic policy and process-supervisor expectations.

7. Capacity and backpressure
   - Replace informational `worker_threads` or remove the setting.
   - Establish maximum events/s, concurrent connections, frame size, memory ceiling, WAL throughput, and recovery time.
   - Run sustained load with accepted, rejected, shed, queue, latency, and RSS assertions.

### P2 — blocks a credible GA claim

8. Operational observability
   - Publish Prometheus scrape configuration and alerts.
   - Alert on health degradation, XDP inactive, frame rejects, event shedding, queue growth, WAL lag, RAM pressure, and reconciliation drift.
   - Add dashboards for rates and cumulative totals with explicit units.

9. Release governance
   - Add versioning and changelog enforcement.
   - Add signed release artifacts and SBOM.
   - Add dependency and license audit gates.
   - Add a release checklist tied to the metric keystore and deployment manifest.

10. Security and resilience validation
    - Fuzz protocol deserialization and auth/replay paths.
    - Run third-party security review.
    - Test malformed frames, credential rotation, replay, oversized frames, connection exhaustion, disk exhaustion, and restart storms.

## Roadmap

### Phase 0 — Baseline freeze

Goal: make every later result attributable.

Deliverables:

- Source-controlled readiness ledger using this document.
- Versioned production config with no secrets committed.
- One isolated verification environment.
- Baseline snapshot, health response, active PID, ports, image/package identity, and trace log.
- Replace stale roadmap count (`39`) with measured debt count (`49`) or generate the count automatically.

Exit criteria:

```text
git status clean
one daemon
known commit/config identity
healthz OK
review pipeline PASS
baseline report archived
```

### Phase 1 — Security boundary

Goal: make external exposure safe.

Work:

1. Define secret injection for IPC HMAC and dashboard auth.
2. Add production config validation: public bind without auth/TLS fails startup.
3. Add key rotation and replay-store persistence policy.
4. Add negative/positive auth integration tests.
5. Document trusted-proxy and TLS termination requirements.

Exit criteria:

```text
unauthenticated IPC rejected
replay rejected
credential rotation works without stale consumers
public-bind unsafe configuration refuses startup
security test suite green
```

### Phase 2 — Durability and recovery

Goal: no silent loss of active enforcement state.

Work:

1. Enable WAL in the production profile.
2. Add startup replay and XDP reconciliation assertions.
3. Add corruption, permission, disk-full, and partial-write tests.
4. Add recovery metrics and structured failure reasons.
5. Execute kill/restart drills under active blocking traffic.

Exit criteria:

```text
block survives restart
TTL state behaves correctly after restart
WAL failure is visible and policy-defined
XDP map converges after restart or map loss
recovery time measured
```

### Phase 3 — Deployable artifact

Goal: repeatable installation and rollback.

Work:

1. Add minimal multi-stage Dockerfile or explicitly choose host-package deployment.
2. Add OCI labels: revision, version, keystore hash, schema hash, build time.
3. Pin base image by digest.
4. Add SBOM and vulnerability scan.
5. Add systemd/container manifest with capability allowlist, resource limits, ports, volumes, and health checks.
6. Build, start, probe, stop, and rollback in CI.

Exit criteria:

```text
immutable digest exists
fresh host starts from documented command
health/readiness pass
config identity matches source
rollback restores previous artifact
no manual capability step outside deployment automation
```

### Phase 4 — Convergence and capacity

Goal: stay correct under sustained load and partial failure.

Work:

1. Implement periodic enforcement reconciliation.
2. Resolve single-thread processing semantics.
3. Run sustained and burst load profiles on scratch ports.
4. Measure p50/p95/p99 ingestion-to-enforcement latency.
5. Establish RAM, queue, WAL, connection, and shed thresholds.
6. Exercise XDP traffic separately from IPC traffic.

Exit criteria:

```text
capacity envelope documented
no unbounded queue growth
shed behavior intentional and observable
reconciliation drift returns to zero
SLO thresholds have repeatable tests
```

### Phase 5 — Release and operations

Goal: support unattended operation.

Work:

1. Add Prometheus alerts and operator runbooks.
2. Add signed release and changelog gates.
3. Remove or justify the 439 production `.unwrap()`/`.expect()` matches.
4. Run fuzzing and security review.
5. Perform a game day: attack, alert, degrade, restart, rollback, recover.

Exit criteria:

```text
operator can detect failure without SSH
operator can recover without source changes
rollback drill passes
security review findings triaged
release checklist complete
```

## Findings ledger

Open findings tracked during roadmap execution. Each entry: evidence, contract
mismatch, owner, phase, status. Close an entry only with a verification report
and rollback reference.

### F5 — subnet batch blocks cover a fraction of the /24 (P1)

- **Date**: 2026-09-17
- **Evidence**: trace probe (`RUST_LOG=ramshield=trace`, scratch server,
  env recipe thresholds 1/1): `Batch block subnet in window cidr=192.0.2.0/24`
  fired on 4 successive scans (unique_ips=113→167→171→178) yet only **17 IPs**
  were committed (`reason=subnet_burst`, `ttl_seconds=120`, `xdp_applied=true`);
  snapshot `blocks_applied=17`. `check_ip` on any other fixture host → unblocked.
- **Root contract mismatch**: `EnforceCommand.ip` is an exact `IpAddr` — there is
  no CIDR command. The scan iterates `get_ips_in_subnet_windowed(sk, 2s)` and
  blocks each unblocked host individually, but the windowed index/loop yields a
  sparse subset per tick. README, docs, and block history label the feature
  "Batch block /24", implying full-subnet coverage.
- **Impact**: ~90% of hosts in the /24 evade the "subnet" block; a rotating
  attacker within the range stays protected. Security-relevant.
- **Owner**: `crates/ramshield-detection/src/lib.rs` `subnet_batch_scan` +
  `crates/ramshield-storage/src/lib.rs` `get_ips_in_subnet_windowed`.
- **Fix**: `EnforceCommand` gained a `cidr: Option<IpNetwork>` variant
  (`crates/ramshield-types/src/command.rs`); enforcement layers a true prefix
  block onto XDP `BLOCKCIDR`/`BLOCKCIDR6` LPM trie maps; IPC `BlockCidr`
  request validates and normalizes prefixes (`src/ipc/server.rs`).
  Full-prefix coverage is now structural — one map entry covers every host,
  independent of the windowed per-IP index.
- **Verified**: `scripts/xdp_netns_sim.py --cidr 203.0.113.0/24`, isolated veth
  pair + 2 netns VMs broadcasting at ~115k pps, XDP counters before/after:
  `wire_pass 5→10` (ARP only), `v4_drops 0→919408` — 100% of 919,408 packets
  from the blocked prefix dropped, unblocked traffic passes.
- **Phase**: convergence & security (roadmap Phase 4/5).
- **Status**: closed in commits `803460e` (LPM maps), `e7805a9` (IPC path),
  `b4b3df4` (netns packet verification), verified 2026-09-17.

### F1 — production `.expect()` in detection SHM boot path

- **Date**: 2026-09-17
- **Evidence**: `audit_static` no-unwrap gate flagged two boot opens at
  `crates/ramshield-detection/src/lib.rs`.
- **Fix**: `DetectionEngine::try_new` now returns `std::io::Result<Self>`;
  boot propagates SHM initialization failure. The second SHM open was removed;
  both consumers share one initialized `Arc<ShmTableManager>`. Test-only
  compatibility constructor remains outside the production boot path.
- **Verified**: `final_integration.py` audit_static 13/13; release build;
  full integration SUCCESS.
- **Status**: closed in commit `2f55ea9`.

## Verification matrix

| Boundary | Current evidence | Required next proof |
|---|---|---|
| Rust source | check, Clippy, tests pass | production panic audit |
| Metric contract | keystore + JSONL validated | enforce all public metrics have live evidence |
| IPC | live ingestion and frame rejection counter | authenticated external-client test |
| Detection | full metrics verification | sustained capacity and latency envelope |
| XDP | live `xdp_active=true`, one wire pass | privileged drop/pass matrix and map-loss recovery |
| Dashboard/SSE | live snapshot and telemetry checks | auth, browser-level served-asset test |
| Storage | code path exists | kill/restart/WAL corruption drills |
| Logs | trace auditor and coverage probe | alert mapping and retention policy |
| Deployment | local binary only | immutable artifact, supervisor, rollback |
| Operations | manual scripts | one-command deploy/rollback/runbook |

## Change order

Do not start with UI polish or performance tuning. Execute in this order:

```text
security boundary
  -> durability/recovery
  -> deployable artifact
  -> supervision
  -> reconciliation
  -> capacity
  -> alerts/runbooks
  -> release hardening
```

Each phase gets one atomic change per finding, one RED-capable check, one verification report, and one rollback reference. No phase is complete because the compiler is green; completion requires source, runtime, telemetry, logs, and deployment evidence to agree.
