# RamShield Engineering Constitution

Status: design (audit-branch). Not an implementation patch.
Target: master-11 working tree at `dfa7d95`.
Rule: every later patch must leave an audit trail in source. The runbook
(`docs/LOCKDOWN_RUNBOOK.md`) is the constitution, not the Rust fix.

This document is the rebuild contract. Implementation happens as numbered
source patches, one subsystem at a time, each with a stop gate.

---

## 0. Central rule

Every subsystem must explain its own behavior in the source, expose its
invariants, and leave an audit trail for the next engineer.

Comments are literal operational documentation, not rationale.

Wrong:

    // Fixes ARM64 race.

Right:

    // Marks the slot as being written so readers ignore its payload.
    slot.seq.fetch_add(1, Ordering::Release);

Four allowed comment kinds on non-obvious modifications:

1. Literal — what the next statement does.
2. Invariant — who owns state; what must remain true.
3. Boundary — untrusted input converted into a bounded internal form.
4. Unsafe-boundary — exact ABI/width/clock the unsafe block depends on.

---

## 1. Ownership (the simplification)

Authoritative chain:

    CONFIG → RECOVERY (ramwal) → STORE → Enforcement coordinator
                                           ├── XDP  (projection)
                                           ├── SHM  (projection)
                                           └── MESH (projection)

MUST NOT mutate XDP / SHM / kernel maps:

- detection
- mesh
- IPC
- native ingest
- upstream
- forecasting

They emit `EnforceCommand` / authenticated events only.

Enforcement is the single writer: WAL append → store mutation → TTL →
projection. Detection does not own the dataplane.

Input:

    NETWORK ── XDP ─────────────────────────┐
             └── AF_PACKET → bounded events ┴→ DETECTION → EnforceCommand
                                                       → ENFORCEMENT

    IPC  ──┐
    MESH ──┼── authenticated command/event boundary (limits, replay, HMAC)
    NATIVE─┘

---

## 2. Failure policy (explicit)

             FAILURE
                │
       ┌────────┼────────┐
       ▼        ▼        ▼
     FATAL   DEGRADED  RECOVERABLE
       │        │        │
    shutdown  continue  restart/reject

| Component        | Policy |
|------------------|--------|
| WAL recovery     | Fatal |
| Enforcement actor| Fatal |
| Detection workers| Fatal if workers fail to start; else config |
| XDP projection   | Policy (`allow_inband_fallback`) |
| Native ingest    | Config |
| SYNPROXY         | Fatal when enabled and install fails |
| IPC              | Restart/reject connection |
| Mesh             | Degraded |
| Upstream         | Degraded |
| Metrics          | Non-fatal |
| SHM              | Degraded/fallback |

No production constructor panics because an external resource is missing.
No production `unimplemented!()`, `todo!()`, or unproven `unwrap()`.

---

## 3. What is true on this tree (verified, not assumed)

Branch: `audit-branch` @ `dfa7d95`. Isolated from
`patch-fix-0.6-3026010007`. Nested `rs/` tree is accidental zip nesting;
ignore it.

### P0-A — boot deadlock (confirmed)

`src/engine/boot.rs` still inlines:

    enforcement.run(enforcement_rx).await

Comment claims this avoids a Send bound from mesh types. That comment is
stale.

Verified:

- `EnforcementService` as a struct **is** `Send + 'static`
  (`assert_send::<EnforcementService>()` compiles).
- `XdpApplier: Send + Sync`.
- `MeshHandle` fields are `Arc` + `tokio::sync::Mutex` (Send).
- `enforce()` takes `let _ckpt_guard = checkpoint_shared.barrier.lock()`
  (`std::sync::MutexGuard`, `!Send`).
- Block path: `drop(_ckpt_guard)` at `enforce.rs:218`, then
  `handle.broadcast(...).await` at `:263`. Guard is gone before await.
- Unblock path: **never drops** the guard, then
  `handle.broadcast(...).await` at `:321`. Guard is live across await.
- Therefore `EnforcementService::run()`'s future is `!Send`.
- `tokio::spawn` of `run()` fails with E0277 naming
  `apply_mesh_block()` because that future calls `enforce()`.

The inline `.await` is a workaround, not architecture. It starves
detection, native ingest, SYNPROXY, IPC, readiness.

Correct shape (runbook §1, Gotcha C) after the guard is scoped:

    let mut handle = tokio::spawn(async move { enforcement.run(rx).await });
    // continue boot: detection, native, synproxy, ipc
    tokio::select! {
        res = &mut handle => { /* FATAL: pipeline_failed */ }
        _ = shutdown_rx.changed() => { /* join with timeout */ }
        _ = server.start() => {}
    }

Do **not** `unsafe impl Send`. Scope the std mutex so it cannot live
across `.await`. Shortest diff.

### P0-B — SHM torn seed (confirmed)

`crates/ramshield-cgnat/include/ramshield_shm.h`:

- `challenge_seed[16]` is an ordinary `uint8_t` array.
- Reader copies bytes under seqlock; writer (Rust) still a byte array.
- `_Static_assert(sizeof == 128)` exists.
- Layout today: seq@0, pad, client_hash@8, expires@16, max_rps@24,
  tier@26, flags@27, seed@28, pad to 128.

Runbook wants two `_Atomic uint64_t` seed words, naturally aligned,
C and Rust identical, no relaxed final seq load.

Do **not** invent offsets. Measure `offsetof` / `size_of` / `align_of`
on both sides in the same patch, then change.

`ramshield_ipv4_subnet_key`: `prefix_len == 0` is guarded; `prefix_len > 32`
is **not**. `<< (32 - prefix_len)` is UB in C for prefix_len > 32.
Reject, do not clamp (48 → 32 would transmute firewall policy).

### P0-C — replay optional (confirmed)

`crates/ramshield-protocol/src/auth.rs`:

    pub fn verify(..., replay: Option<&ReplayStore>)

`verify_authenticated` requires `&ReplayStore` and delegates with
`Some(replay)`. Public `verify` still allows omission.

Pipeline order in `verify` is already: skew window → constant-time HMAC
→ `check_and_record`. Good. Do not record before HMAC.

Gotcha D: cap store (`max_nonces_per_key`) and shrink skew (30s → 10s)
so a legitimate 50k req/s signer cannot grow the map to ~96 MB.

`Lsn::next()` still panics on exhaustion (`crates/ramwal/src/lsn.rs:40`).
`checked_next()` exists. Callers of `next()` are the panic surface.

### P0-D — XDP unimplemented (confirmed)

`crates/ramshield-enforcement/src/xdp.rs:388-394`:

    configure_trusted_overlay → unimplemented!()
    configure_autonomous      → unimplemented!()

Trait in `applier.rs` already demands both. Production baseline
(`config.baseline.toml`) can enable autonomous and abort at boot.

Do **not** write map code until BPF object map names and key/value
layouts are inspected. Fail `MapNotFound` / `MapWrite`. Never `Ok(())`
after skipping IPv6.

### P0-E — systemd vs SYNPROXY (runbook; not yet measured in unit files)

`ProtectKernelTunables=true` on `ramshield.service` fights SYNPROXY
sysctls. Companion oneshot or `/etc/sysctl.d/` — do not fold sysctl
writes into the unprivileged daemon.

### P0-F — replay RAM (depends on P0-C)

Cap after HMAC-only insert is proven.

### Nested tree

`rs/` inside this repo is a full second copy from zip extract. Delete
in a hygiene patch. Do not edit both.

---

## 4. Rust-masters gaps (undercooked)

Commandments vs this tree:

| Commandment | Gap |
|-------------|-----|
| Ownership is the API | Enforcement owns store; XDP trait still has `unimplemented!()` so ownership is fiction at boot |
| Errors are values | `Lsn::next` panics; `verify(..., Option)` lets callers drop replay |
| Test concurrency | no `loom`; seqlock untested on AArch64 |
| Benchmark before optimizing | benches exist; `criterion` not in root `Cargo.toml` as a declared gate |
| Minimal public API | `verify` remains the leaky twin of `verify_authenticated` |
| Docs that compile | runbook is markdown-only; no doctest of the seqlock protocol |
| Tooling is the language | clippy `-D warnings` is the gate; keep it |

YAGNI for this rebuild: no new lifecycle crate, no actor framework, no
new map type. Scope the mutex. Spawn. Select. Delete `unimplemented!()`
in favor of real map writes or explicit `Err`.

---

## 5. Sequenced patches (no god patch)

Each patch:

    SOURCE → COMMENT (one of the four kinds) → UNIT TEST →
    INTEGRATION TEST → STATIC CHECK → REVIEW → NEXT

Stop gate every time:

    cargo test --workspace --locked --all-targets --features full
    cargo clippy --all-targets -- -D warnings

### 01-lifecycle-send

Files: `crates/ramshield-enforcement/src/service/enforce.rs`,
`src/engine/boot.rs`.

1. Drop `_ckpt_guard` on **every** path before any `.await`
   (Unblock currently misses this). Comment: barrier protects WAL+store
   only; mesh broadcast is outside the barrier.
2. Spawn `enforcement.run` at boot; retain `JoinHandle`.
3. `select!` actor crash → `pipeline_failed` (FATAL).
4. Shutdown joins the handle with timeout (Gotcha C).
5. Test: boot reaches detection construction without awaiting `run()`.
6. Compile-time: `fn assert_send<F: Send>(_: F) {}` on `svc.run(rx)`.

Skipped until this is green: everything else that assumes a live daemon.

### 02-detection-init

Confirm `DetectionEngine::try_new` is the only production constructor.
No panicking `new()` on the boot path. Test: SHM/init error is `Err`,
not abort.

### 03-xdp-inventory (no behavior change)

Inspect loaded BPF object. Document map names, key/value sizes, in
`docs/CAPABILITY_MATRIX.md`. If trusted-overlay / autonomous maps are
absent, that is a capability miss — boot must fail closed when config
enables them. Do not stub `Ok(())`.

### 04-xdp-overlay

Implement `configure_trusted_overlay` against real maps. IPv4 and IPv6.
Any failed insert → error. No silent skip.

### 05-xdp-autonomous

`window_ms > 0`, checked `window_ms * 1_000_000`, write array after
validation. Missing map → `MapNotFound`.

### 06-shm-abi-measure

C `offsetof`/`sizeof`/`_Static_assert` and Rust `offset_of!` printed
side by side. No layout change in this patch.

### 07-shm-atomic-seed

Replace `challenge_seed[16]` with `challenge_seed_lo/hi` atomics.
Writer: odd seq (release) → payload → release fence → even seq (release).
Reader: acquire seq, reject odd, atomic seed loads, acquire fence,
acquire seq again. Reject `prefix_len > 32` in C and Rust.
Concurrency test x86_64; AArch64 is a signoff gate, not optional.

### 08-protocol-replay-mandatory

Make `verify` require `&ReplayStore`. Keep `verify_authenticated`.
Call sites that passed `None` must construct a store or fail closed.
Cap + 10s skew (Gotcha D). Test: unsigned traffic cannot insert.

### 09-wal-lsn-no-panic

`next()` returns `Option<Lsn>` or callers switch to `checked_next`.
No process panic on identity exhaustion.

### 10-systemd-synproxy

Oneshot `ramshield-synproxy-init.service` or sysctl.d. Daemon stays
non-root with `ProtectKernelTunables=true`.

### 11-hygiene-nested-rs

Delete nested `rs/` copy. One tree.

Later: IPC flood envelope, TPACKET_V3 shutdown, mesh replay separate
from IPC replay, WAL disk-full tests.

---

## 6. Integration junctions (what "tested" means here)

A junction is tested when both sides of a boundary have one check:

| Junction | Proof |
|----------|--------|
| Detection → EnforceCommand → Enforcement | command applied, WAL LSN set, XDP called |
| Enforcement → XDP maps | reconcile vs store; MapNotFound on missing |
| Enforcement → SHM | seqlock: torn seed never observed |
| IPC → protocol verify | HMAC then nonce; omit store does not compile |
| Mesh → Enforcement | mesh cannot unblock operator suppression |
| WAL → Store on boot | recovery before projections |
| Boot → readiness | enforcement JoinHandle live before `/healthz` protected |
| Shutdown | every actor joined or timed out; SYNPROXY uninstalled |

Thresholds (detect 50 IPs + 100 events /24/2s, replay 10s, IPC 256
conns / 256 KiB / 64 KiB frame) live in `validate.rs` and have a
failing test if a default drifts.

---

## 7. What this constitution refuses

- Clamping invalid prefixes.
- Recording nonces before HMAC.
- `Ok(())` for unimplemented dataplane features.
- `unsafe impl Send` to silence the boot spawn.
- A single `ramshield-final-fixed.patch`.
- Editing the nested `rs/` copy.
- Merging onto `patch-fix-0.6-3026010007` until 01-lifecycle-send is green
  on this branch.

---

## 8. First implementable unit (when execution resumes)

01-lifecycle-send only:

- `enforce.rs`: drop checkpoint guard before mesh `.await` on Unblock
  (and any other path). One comment: barrier is WAL+store, not mesh IO.
- Compile assertion: `run()` future is `Send`.
- `boot.rs`: spawn + retain handle + select crash/shutdown.
- One test: `run()` future satisfies `Send` (the assert is the test).

Then stop. Do not start SHM or XDP in the same commit.
