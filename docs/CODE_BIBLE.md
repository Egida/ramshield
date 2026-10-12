# RamShield code bible

Maintainer contract refreshed against the implementation on 2026-10-12. This
records source-level invariants; deployment qualification and runtime health
must still be checked on the target host.

This document is the maintainer contract. The objective is not maximum abstraction; it is maximum correctness with the smallest number of moving parts.

## 1. Core authority rule

Userspace block state is authoritative. When WAL is configured, it is the
durable log for that state. XDP and CGNAT shared memory (SHM) are projections;
never treat a projection as the source of truth. Without WAL, the running
process has no WAL-backed restart durability guarantee.

```text
telemetry
  → detection
  → enforcement decision
  → configured checkpoint barrier
  → WAL append (when configured)
  → userspace state
  → projection (XDP/SHM)
```

A projection failure must be observable without corrupting authoritative state.

## 2. Crate ownership

| Crate | Responsibility |
|---|---|
| `ramshield-types` | shared domain types and commands |
| `ramshield-config` | TOML schema, defaults, environment overrides, validation |
| `ramshield-detection` | event batching, promotion, detection decisions |
| `ramshield-forecasting` | statistical/forecasting signals |
| `ramshield-storage` | authoritative IP/CIDR store, checkpoints, recovery adapter |
| `ramwal` | standalone durable WAL implementation |
| `ramshield-enforcement` | authoritative enforcement orchestration and projection |
| `ramshield-xdp` | XDP/BPF loading and dataplane integration |
| `ramshield-cgnat` | fixed-size SHM projection and CGNAT integration |
| `ramshield-protocol` | IPC messages, authentication and replay defense |
| `ramshield-metrics` | metrics, snapshots and health state |
| `ramshield-analytics` | analytics primitives |
| `ramshield-mesh` | experimental multi-node functionality |
| root `engine` | runtime composition, startup, recovery and orchestration |
| root `ipc` | server boundary |
| root `dashboard` | operator HTTP surface |

## 3. Enforcement ordering

The security-critical ordering is:

```text
validate
  ↓
durability/checkpoint barrier
  ↓
WAL append
  ↓
userspace mutation
  ↓
projection attempt
  ↓
projection-health update
```

Do not move XDP mutation inside the durability transaction merely to make the
code look synchronous. Kernel projection is not the authority. A committed
userspace block is not proof that XDP applied it; inspect the projection result
and health. XDP startup fallback is configuration-dependent.

## 4. XDP cursor invariant

The reconciliation cursor is retained-key-only:

```rust
prev_key = None
// ...
if stale { delete(k) } else { prev_key = Some(k) }
```

`prev_key` must never reference a deleted key. Do not replace this with collect-then-delete unless the kernel API contract changes and a proof demonstrates that the current invariant is insufficient.

## 5. XDP freshness invariant

When active XDP mutation fails, projection health becomes stale immediately. Successful reconciliation clears stale. Do not add a second state machine, duplicate timestamp, or redundant atomic.

## 6. Detection invariants

- Absolute detection remains the default contract.
- Relative detection is opt-in.
- Relative detection evaluates against the prior slow baseline.
- The baseline is frozen during an active breach streak.
- Hysteresis is explicit; a single noisy sample must not silently become a sustained breach.
- Configuration values used in floating-point decisions must be finite and domain-valid.
- Keep optional relative detection disabled until qualified against
  representative traffic; absolute detection remains the baseline contract.

## 7. WAL invariants

- Corrupt final tails may be repaired only under the documented final-segment rule.
- Non-final corruption is not silently repaired.
- Durable checkpoint publication is atomic.
- Directory durability matters after rename.
- New state files are owner-only.
- Recovery reconstructs snapshot plus verified WAL tail.

## 8. SHM invariants

- Fixed-size table.
- Four-slot bounded probe.
- Existing matching key is preferred.
- No silent eviction of unrelated live rules.
- Rust writer and C reader share the same ABI and probe contract.
- TTL arithmetic saturates.
- Projection saturation is a projection failure, not authoritative-state loss.

## 9. Rust style

Prefer boring Rust:

- ownership over shared mutable state where practical;
- `Result`/`Option` for expected failure;
- explicit bounds;
- small functions around invariants;
- no speculative abstraction;
- no allocation in a hot path unless measured;
- `unsafe` only at explicit OS/BPF/ABI boundaries;
- comments explain **why**, not what the syntax already says.

Do not enable all of Clippy's restriction/pedantic lints blindly. The project uses `-D warnings` for the selected lint set; Clippy itself notes that restriction lints can conflict with reasonable code.

## 10. Runtime and qualification assumptions

- Telemetry availability and representativeness are deployment properties.
  Source-level support does not prove that a proxy or packet source is wired in.
- A healthy process is not proof of active XDP enforcement, durable WAL, or
  healthy SHM projection. Check each required boundary independently.
- XDP capabilities depend on kernel, driver/mode, privileges, map capacity, and
  configuration. A failed optional projection must remain visible.
- Dashboard and IPC authentication do not encrypt transport. Put externally
  reachable management interfaces behind TLS/mTLS; a cookie security flag is
  not wire encryption.
- Mesh code is experimental and does not imply production consensus or
  multi-node acceptance. Follow the separate mesh qualification plan.
- Detection thresholds, memory limits, queue capacity, and event rates require
  target-specific load and false-positive qualification.

## 11. Change protocol

Every security-sensitive change must have:

1. invariant statement;
2. minimal implementation change;
3. regression test;
4. failure-path test where applicable;
5. documentation update;
6. release/qualification impact assessment.

Never claim a runtime property from source inspection alone.
