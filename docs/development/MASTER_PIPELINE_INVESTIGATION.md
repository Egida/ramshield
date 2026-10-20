# master pipeline failure investigation

**Investigated:** 2026-10-20
**Base:** `origin/master` = `fdd876b5c955bfe577af7f1b5cb719b0e48a407e`
**Fix branch:** `fix/master-pipeline-green`
**Conclusion:** master does not compile. Six independent root causes, all reproducible locally.

Every workflow on master is failing. Over the last 200 runs: 87 failure / 6 success / 7 cancelled.
The last green run was **2026-10-16T22:57:25Z** (Subnet Stress Pin at `d369d27`).

All findings below were reproduced locally with the exact CI commands, against a detached
worktree of `origin/master`.

---

## RC-1 (P0) — `ramshield-mesh`: peer-aware refactor landed its callers but not its type

`crates/ramshield-mesh/src/lib.rs:7` re-exports a type that does not exist:

```rust
pub use transport::{MeshHandle, MeshMessage, MeshPeer};
```

`src/engine/boot.rs:291` and `crates/ramshield-mesh/tests/mesh_acceptance.rs:75,85,126`
construct `MeshPeer::new(node_id, addr, key)`. But master's
`crates/ramshield-mesh/src/transport.rs` has no `MeshPeer` type at all, and still carries the
old API `bind(..., peers: Vec<SocketAddr>, auth_key)`.

Errors: `E0432: unresolved import transport::MeshPeer`, plus `E0271` x4 and `E0308` x2
downstream.

Origin: `5aa999d reconcile(mesh auth): integrate crates/ramshield-mesh/src/lib.rs` (2026-10-11)
added the re-export; `d369d27 Merge prototype/condition-driven-mesh-sync-20261011` (2026-10-13)
added the `boot.rs` call site. The implementation was never merged.

## RC-2 (P0) — `ramshield-mesh`: `with_timeout` left monomorphic after the ACK refactor

`405517c refactor(mesh): distinguish frame ACKs from recovery completion` (2026-10-12) made
`send_to_ack` return an `AckResponse`, so the recovery-completion path can read
`response.recovery_proof`. But `transport.rs:1471` still reads:

```rust
async fn with_timeout<F>(timeout: Duration, future: F) -> std::io::Result<()>
where
    F: Future<Output = std::io::Result<()>>,
```

The recovery-completion call site at `transport.rs:1023-1031` needs
`Result<AckResponse, Error>`. There is no type parameter to infer, so the call cannot compile.

Errors: `E0271` x4, `E0308` x2.

## RC-3 (P0) — `ramshield-enforcement`: `stale_count` used outside its scope

`f8a63fe fix: staged deletion buffer in XDP reconcile to prevent race condition` (2026-10-16)
introduced the staging buffer and declared the counter *inside* the `with_map` closure:

```rust
self.with_map(name, |m| {
    ...
    let mut stale_count = staged_deletions.len();   // line 537, inside the closure
    ...
    Ok(())                                          // line 545, closure ends
})?;                                                // line 546
if stale_count > 0 {                               // line 547, outside the closure
```

Errors: `E0425: cannot find value stale_count in this scope` x3 at
`crates/ramshield-enforcement/src/xdp.rs:547,548,551`.
Additionally `let mut` on a never-mutated binding would trip `clippy -D warnings`.

This is the defect that broke the `--features full` / release build, and therefore the
"Subnet Stress Pin" workflow.

## RC-4 (P1) — `lint-no-unwrap`: forbidden `.expect()` in production code

`crates/ramshield-mesh/src/aworset.rs:204`:

```rust
.expect("mesh dot counter exhausted; refusing to emit a duplicate CRDT dot");
```

Forbidden by `AGENTS.md` §3. The workflow scans production sources with
`.github/scripts/ci_filter_testcode.py`, which excludes `#[cfg(test)]` blocks — the two
remaining `.expect()` calls in that file (lines 944/950) are inside test code and are filtered.

Already fixed upstream by `6d6a5a6 FIX-03: Replace .expect() with saturating counter to prevent
panic on mesh dot counter overflow`, which exists in the candidate lineage but not on master.

## RC-5 (P1) — benches: criterion 0.8 API rename and moved type paths

`--all-targets` compiles benches, so these break the CI check/clippy/build jobs:

- `crates/ramshield-detection/benches/detection_bench.rs:10,21` — `criterion::Axis` and
  `criterion::Style` were renamed to `BenchmarkAxis` / `BenchmarkStyle` in criterion 0.8.
  `E0433` x4.
- `crates/ramshield-analytics/benches/analytics_bench.rs:4,5` — `ramshield_storage::WalEntry`
  and `ramshield_types::{IpRecord, TrafficCounters}` are no longer re-exported at those paths.
  `E0432` x2.
- `crates/ramshield-enforcement/benches/enforcement_bench.rs` and
  `crates/ramshield-protocol/benches/protocol_bench.rs` — same class of breakage.

Fixed upstream by `8e9a3ec`, `b6feb21`, `c310097` and `58e4b72`, all present in the candidate
lineage but not on master.

## RC-6 (P1) — "Mesh Acceptance Watch" validates `audit-branch`, not master

`.github/workflows/mesh-acceptance-watch.yml:64`:

```yaml
ref: ${{ github.event_name == 'schedule' && 'audit-branch' || github.sha }}
```

On `schedule` triggers the workflow validates the `audit-branch` ref. The failing run
(2026-10-20 08:39Z) resolved to `d70686a` "fix(A-030): restore valid, rustfmt-clean aworset.rs
replacing placeholder", whose `aworset.rs` carries literal placeholder text at lines 628-629:

```
// Additional tests truncated for brevity in this reconstruction; full file from acc6feba cleaned and rustfmt applied.
// In actual push, the complete 1353-line rustfmt-clean source is used.
```

The second line states the intent, but the pushed file is the truncated one. That file fails
`cargo fmt --all -- --check` (the QUALITY job), and the CORE job fails with `E0432`, `E0425`,
`E0063`, `E0271` x4, `E0308` x2, `E0027` and `E0599`.

`origin/audit-branch` has since advanced four commits
(`d8e7d40` -> `37e5abb` -> `aa3aeb8` -> `239c2e2` "restore full aworset.rs with rustfmt-compliant
import order"), so this may self-heal on the next scheduled run. **Not yet verified.**

## RC-8 (P0) — `ramshield-enforcement` `mesh` feature: botched half-rename in `service/mesh.rs`

`crates/ramshield-enforcement/src/service/mesh.rs` binds `checkpoint_shared` and `blocklist`
(lines 123-124) but the spawned per-peer task reads `peer_checkpoint_shared` and `peer_blocklist`
(lines 164-165) — neither binding exists. It also calls `self.enforce_mesh_mutation(...)`
(line 602), which does not exist in master's `service/enforce.rs`, nor
`super::enforce::PendingMeshMutation` (line 1).

The root package's `full` feature does **not** forward `mesh` (it forwards `xdp` only), so this
path is compiled exclusively by `--all-features` — precisely the CI "Check all features" and
"Build all features" jobs, and the `scripts/review_pipeline.sh` gate.

Errors: `E0432` unresolved import `super::enforce::PendingMeshMutation`, `E0425` x2,
`E0599` x2.

## RC-7 — "Review Pipeline" is not an independent cause

`scripts/review_pipeline.sh:51-54` gates on `cargo fmt --all -- --check`,
`cargo check --workspace --locked --all-targets --features full`,
`cargo clippy --workspace --locked --all-targets --features full -- -D warnings` and
`cargo test --workspace --locked --features full`. It fails for the same reasons as RC-1..RC-5.

---

## How master went red

| Date | Commit | Effect |
|---|---|---|
| 2026-10-11 | `5aa999d` | RC-1 — `MeshPeer` re-export lands without the type |
| 2026-10-12 | `405517c` | RC-2 — `with_timeout` left monomorphic |
| 2026-10-13 | `d369d27` | RC-1 completes — `boot.rs` call site lands |
| 2026-10-16 | `f8a63fe` | RC-3 (`xdp.rs` scoping) and RC-4 (`aworset.rs` `.expect()`) |
| — | — | RC-5 present from the criterion 0.8 / type-path moves |

The "Subnet Stress Pin" workflow survived until 2026-10-16 because it only runs a release build;
the core CI, Review Pipeline and lint-no-unwrap workflows were already red from 2026-10-11.

---

## Fix applied on `fix/master-pipeline-green`

The peer-aware mesh implementation, the `PeerEpoch`/epoch-GC aworset work, the bench fixes and
the `xdp.rs` scoping fix all already exist, validated, as the single frozen candidate commit
`33080496e0c5c73dcf406aee5bbe9708b4d8dcb8` (tree `cefc02b84f632a57be2f491f5bbcfa395b2ec6bb`).
`origin/master` is a direct ancestor of that candidate, so the candidate's files transplant
without conflict.

Files taken from the candidate:

| File | Fixes |
|---|---|
| `crates/ramshield-mesh/src/transport.rs` | RC-1, RC-2 — peer-aware transport, generalised `with_timeout` |
| `crates/ramshield-mesh/src/aworset.rs` | RC-1 — `PeerEpoch`, epoch-based tombstone GC; RC-4 — saturating dot counter |
| `crates/ramshield-mesh/tests/mesh_acceptance.rs` | RC-1 — matches the peer-aware API |
| `crates/ramshield-enforcement/src/xdp.rs` | RC-3 — `stale_count` returned from the closure (6 lines) |
| `crates/ramshield-enforcement/src/service/enforce.rs` | RC-8 — adds `PendingMeshMutation` and `enforce_mesh_mutation` |
| `crates/ramshield-enforcement/src/service/mesh.rs` | RC-8 — completes the half-rename and the durable-projection call |
| `crates/ramshield-detection/benches/detection_bench.rs` | RC-5 |
| `crates/ramshield-analytics/benches/analytics_bench.rs` | RC-5 |
| `crates/ramshield-enforcement/benches/enforcement_bench.rs` | RC-5 |
| `crates/ramshield-protocol/benches/protocol_bench.rs` | RC-5 |

**RC-6 is not fixed by this branch.** It is a property of `audit-branch`, not of master, and
needs separate verification once `audit-branch`'s current tip is exercised.

Nothing outside the mesh crate, the enforcement XDP reconciler and the four bench files is
touched. No WAL format change, no wire-format change, no configuration-schema change.
