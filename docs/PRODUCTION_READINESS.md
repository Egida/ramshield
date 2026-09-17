# RamShield Production Readiness Review

Review date: 2026-09-17
Branch: `p1`
Reviewed commit: `ac79c40`

## Verdict

Status: NOT production-ready.

Current state supports a controlled single-node pilot on localhost with XDP capabilities. It does not yet support an unattended, externally exposed, recoverable production deployment.

Readiness score: 5/10.

The core processing path is healthy. The release boundary, durability boundary, security boundary, and operational recovery boundary remain incomplete.

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

- No Dockerfile, OCI image build, image signing, digest promotion, or rollback artifact exists.
- No systemd unit, service supervisor contract, or restart policy is source-controlled.
- `config-xdp.toml` binds localhost, has no active IPC HMAC key, and has no WAL section.
- IPC has no TLS; safe only behind localhost or a trusted private transport.
- Runtime reports `wal_lsn=0`; restart durability is not production-proven.
- Enforcement reconciliation is startup-only; drift after startup is not automatically repaired.
- `worker_threads` is informational; processing remains single-batch-thread.
- XDP requires host capabilities and manual capability restoration after builds.
- Current source contains 49 production `.unwrap()`/`.expect()` matches under `src/`.
- Live/XDP verification is not part of CI; the review pipeline currently skips it by design.
- No measured SLOs, alert rules, capacity envelope, or rollback drill are recorded.
- No authenticated external-control deployment has been exercised.

## Blocker classification

### P0 — blocks any external production exposure

1. Control-plane security
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
   - Add systemd or container orchestration manifest.
   - Define readiness, liveness, graceful shutdown, restart backoff, resource limits, and log retention.
   - Ensure exactly one process owns IPC and dashboard ports.

5. Enforcement convergence
   - Add periodic store → XDP reconciliation.
   - Export reconciliation age, failures, and drift count.
   - Test XDP map loss/reload while the daemon remains alive.

6. Failure handling
   - Remove or justify the 49 production `.unwrap()`/`.expect()` matches.
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
3. Remove or explicitly classify production unwrap/expect sites.
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
