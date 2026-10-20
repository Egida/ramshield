# Master Scan — Performance, Durability and Hardening Fixes

**Base:** `2868250` (the first master-scan push, already on `origin/master`)
**Branch:** `fix/master-p0-p1-fixes`
**Scope:** the non-mesh findings from the five-lens scan, plus every
actionable observation from the independent review of the first push.

Every fix carries a focused regression and was verified RED → GREEN. No test
was weakened to obtain a green suite, and no production default was changed
without the owner asking for it.

---

## P-1 (P1, CPU) — the XDP reconcile re-wrote the entire kernel map every cycle

**File:** `crates/ramshield-enforcement/src/xdp.rs`

The periodic reconcile (every 40 ticks × 250 ms = 10 s) did two things that
are quadratic in the size of the blocklist:

1. It **re-inserted every expected key**, not just the missing ones:

   ```rust
   for k in &expected {
       m.insert(*k, PERMANENT, 0)?;   // bpf_map_update_elem per blocked IP
   }
   ```

   That is one `bpf_map_update_elem` syscall per blocked IP per cycle, and each
   one takes the very bucket lock the XDP **data path** takes on every packet.
   At a large blocklist the periodic reconcile is not just a CPU cost — it is a
   data-plane competitor, and it rewrites keys that are already correct.

2. The CIDR reconcile tested membership with a **linear scan**:

   ```rust
   let is_expected = |k| expected.iter().any(|e| e.prefix_len() == k.prefix_len() && e.data() == k.data());
   ```

   That is O(resident CIDRs × expected CIDRs) — quadratic — every 10 seconds.

**Fix:** the single map walk now yields both halves of the plan. The walk
already visits every resident key, so the absent expected keys are exactly
`expected` minus what was seen:

* `plan.stale` — resident but not expected → delete (unchanged).
* `plan.missing` — expected but not resident → insert.

The reconcile therefore performs **one syscall per resident key instead of
two**, and never rewrites a key that is already correct. The CIDR membership
test became a hash lookup.

**Regression:** `reconcile_plan_inserts_only_absent_expected_keys` — RED with
the old semantics (`left: [0x1000, 0x1002]`, i.e. both expected keys
scheduled for re-insertion; `right: [0x1002]`), GREEN after.

**Value semantics — reviewed, because the independent review flagged it.**

The map value is the **absolute expiry** (`blocklist_value` = `now + ttl`, or
`PERMANENT` for `ttl == 0`), on the same CLOCK_MONOTONIC the eBPF program
reads with `bpf_ktime_get_ns`; the program drops a packet only while
`now < expiry` (`ramshield-xdp-bpf/src/main.rs`). The old reconcile rewrote
**every** expected key with `PERMANENT` every cycle, so the kernel-side TTL
was inert and the userspace owned the expiry.

With the fix, a resident key keeps the expiry `apply_block` gave it. That is
consistent with the rest of the system:

* `apply_block` writes the real expiry;
* the keep-alive re-runs the whole enforce path at the cooldown boundary
  (`ttl/2`), which re-applies the block and so refreshes the kernel expiry
  before it passes;
* `expire_due` reaps the block at the same TTL and issues the unblock that
  removes the map entry.

The `PERMANENT` rewrite was therefore redundant on the normal path. The one
behaviour difference is the corner case where the keep-alive is missed: the
block now lapses at its TTL instead of the reconcile extending it
indefinitely — which is what a TTL is for, and it matches what
`expire_due` does. The semantics are documented in the code at the insert
site.

---

## P-2 (P1, CPU) — the detection merge took a global lock once per IP

**File:** `crates/ramshield-detection/src/pre_aggs.rs`

`merge_local` merged a worker's local aggregate map into the shared
pre-aggregation map **one IP at a time**, taking the global
`pre_aggs_merge_lock` for each IP and releasing it before every
capacity-triggered flush.

Three consequences, all of which bite hardest exactly when the daemon is
under attack:

* every worker serialised on one mutex once per distinct IP it had seen;
* the mutex was held across a **whole map walk** whenever a worker flushed,
  so a flush by one worker stalled every other worker's merge for the
  duration of a million-entry walk (the default `pre_aggs_max_size` is
  **1 000 000**);
* a peer could refill the map between a worker's flush and its retry, so the
  same IP could trigger flush after flush — **O(keys × map size)** for a
  single drain.

**Measured:** with 200 000 distinct IPs in one drain, a competing thread
acquired the merge lock **12 575 552 / 14 141 514 / 13 345 759 times** (three
runs) — about 65 acquisitions per IP. With the fix: **0**.

**Fix:** the drain now takes the lock **once**, for the whole batch. That
also makes the drain atomic with respect to other workers' merges, which is
what the capacity bound needs: no peer can refill the map between two of our
own keys, so the number of flushes is bounded by
`ceil(local.len() / max_entries)` instead of `local.len()`.

The `flushing` CAS gate and its `FlushGuard` were removed: the merge lock
already provides that exclusion, and the CAS was checked *after* the lock was
taken, so it could never fail.

**Regression:** `merge_local_holds_the_merge_lock_for_the_whole_drain` —
RED with the old per-IP lock (the 12.5M count above), GREEN after. The
assertion uses a 1 000-acquisition allowance for the start/end boundary
races, which leaves three orders of magnitude of headroom over the observed
boundary count (≈40) while staying far below the old behaviour.

**New metric:** `ramshield_pre_aggs_flushes_total`
(`Metrics::pre_aggs_flushes`). The flush rate is the direct driver of the
detection path's CPU cost at high IP cardinality, so it is now exported
rather than inferred.

---

## P-3 (P1, durability) — a local block was merged into the CRDT before its WAL record was durable

**Files:** `crates/ramshield-enforcement/src/service/enforce.rs`,
`crates/ramshield-storage/src/wal.rs`

The mesh path has always synced the WAL record before the CRDT merge
(`append_mesh_wal` → `wal.sync()`), so a crash can never leave the CRDT
ahead of the durable log. **The local path did not.** `append_wal` appended
and returned; the record was still in the writer's buffer, or in the OS page
cache, while the decision was merged into the CRDT, written to the store and
applied to the XDP map.

Recovery rebuilds the CRDT and the store **from the log**. So a crash inside
that window silently dropped the block: the operator was told it was applied,
the data plane was enforcing it, and after the restart it was gone.

**Measured:** under `GroupCommit` — the mode that batches, where an append
does not fsync on its own — two consecutive `enforce()` calls left
`durable_lsn() == 1`. The second block's record was **not durable** when
`enforce()` returned.

**Fix:** `append_wal` now syncs before returning, mirroring the mesh path,
with one refinement: `Wal::append_is_durable()` reports whether the mode
already fsynced inside `append` (`SyncEach`), so the hot path does not pay a
second, redundant fsync.

**Regression:** `local_enforce_makes_its_wal_record_durable_before_merging` —
RED (`durable_lsn=1`, 3/3 deterministic), GREEN after.

---

## D-1 (P0, durability) — retention deleted segments that still held required history

**File:** `crates/ramwal/src/wal.rs`

`closed_segments` — the map from segment index to that segment's last LSN — is
seeded at open from the recovery scan:

```rust
let closed_segments: Vec<(u64, u64)> = recovery.report.segment_reports.iter()
    .filter_map(|s| s.last_lsn.map(|l| (s.segment, l.get())))
    .collect();
```

`segment_reports` covers **every** scanned segment, including the newest one —
the segment the writer goes on to use. So the active segment is in the list
from the start, holding its last LSN **as of recovery**.

When that segment later rotates it is pushed onto the list **again**, with its
true (larger) last LSN:

```rust
g.closed_segments.push((closed, closed_last));
```

And the retention decision took the **first** matching entry:

```rust
let Some(&(_, last_lsn)) = closed.iter().find(|&&(s, _)| s == idx) else { … };
if last_lsn >= safe_lsn { continue; }   // keep it
std::fs::remove_file(path)?;             // otherwise delete it
```

`find` returns the seeded entry — the **stale** value. The
`last_lsn >= safe_lsn` guard therefore passed for a segment that actually
held records at or above the safe LSN, and the segment was deleted.

**Consequence: required WAL history silently destroyed.** Recovery rebuilds
the enforcement state from the log, so the records in the deleted segment are
gone: bans that the checkpoint had not yet covered disappear, and the daemon
restarts with them unenforced. This is the project rule's hard case — *"Never
silently skip required WAL history."*

**Reproduced:** with a two-segment WAL (segment 1 holding LSN 1-4, segment 2
holding LSN 5-8 and active), a reopen, three more appends so segment 2 grows
to LSN 9 and rotates, a checkpoint at LSN 9, and one more rotation to
trigger retention — the old code **deleted segment 2**, which still held LSN 9.
The genuinely-old segment 1 was deleted correctly.

**Fix, in two parts:**

1. The decision point now takes the **maximum** last LSN recorded for the
   segment, which is its true last LSN, so the guard keeps any segment that
   still holds records at or above the safe LSN.
2. The bookkeeping now keeps **one entry per segment**: the rotation updates
   the seeded entry instead of appending a duplicate, so the list's invariant
   ("one entry per segment, holding its true last LSN") holds directly.

**Regression:** `retention_keeps_a_rotated_segment_whose_true_last_lsn_exceeds_the_safe_lsn`
— RED with the old code (*"retention deleted the segment that still holds
records at or above the safe LSN (segment 2)"*, 3/3 deterministic), GREEN
after. The test also asserts that the genuinely-old segment *was* deleted, so
it cannot pass vacuously by retention not running.

---

## P-4 (P2) — an unterminated line could buffer past the line limit

**File:** `src/ipc/server/connection.rs`

`max_line_length` was only checked once a newline had been *found*, so the
length of a line that had not ended yet was never checked. With the default
configuration this is harmless, because `max_line_length` and
`max_connection_bytes` are both 1 MiB and the connection byte budget is
checked at the top of the loop — the byte budget is the binding constraint
and the client is correctly told the byte budget was exceeded. The
oversize-connection test pins exactly that.

The gap appears when an operator configures a line limit **below** the byte
budget: then the line limit is the tighter bound, and an unterminated line
could still buffer up to the whole byte budget — 1 MiB per connection, times
up to 1024 connections.

**Fix:** after the newline loop has split off every complete frame, what
remains in the buffer *is* the unterminated tail, and the tail is checked
against the line limit. The check is deliberately applied to the tail and
not to the whole buffer: the buffer can hold several complete pipelined
frames, so its length is not the length of any one line. The check only
applies when the line limit is the tighter of the two bounds, so the
byte-budget error keeps priority in the default configuration and the
existing oversize-connection test is unchanged.

**Regression:** `ipc_wiring_unterminated_line_is_capped_at_the_tighter_line_limit`
— RED without the check, GREEN with it. The pre-existing
`ipc_wiring_oversize_connection_gets_error_frame_not_bare_reset` still
passes, and is the test that pins the byte budget's priority.

## P-5 (P2) — the last unsaturating accumulator in `IpAgg`

**File:** `crates/ramshield-detection/src/batch.rs`

`status_dist[b] += 1` was still a plain increment on a `u32`. Reaching
`u32::MAX` needs ~4.29e9 events of one status class inside a single flush
window, so it is not remotely reachable — but with `overflow-checks = true`
and `panic = "abort"` it would be a process abort rather than a saturating
count, and the fix in the first push had claimed the opposite in its
comment. Now saturating, and the comment is accurate.

---

## P-6 (P2) — an un-repaired torn tail was invisible

**File:** `crates/ramshield-storage/src/wal.rs`

Making the live read paths read-only means a torn tail on the newest segment
is *reported* rather than repaired, and the reader silently sees the valid
prefix until the next boot repairs it. Without a log line that gap is
invisible to an operator. All four read paths now log the tail and the
truncated byte count.

---

## P-7 (P2) — the CGNAT integration test wrote into the production SHM path

**File:** `crates/ramshield-cgnat/tests/cgnat_integration.rs`

The test called `ShmTableManager::default_path()` — the production
`/dev/shm/ramshield_rules`. That means the test and a running daemon share
one table, and (after the first push's ownership check) a file left behind by
an earlier root-owned run makes the test fail. The test now uses a unique
path under the temp dir.

**Found by the independent review** of the first push (observation 9).

---

## P-8 (P2, documentation) — the SHM ownership check's comment overstated what it does

**File:** `crates/ramshield-cgnat/src/shm.rs`

The comment implied the check closed the hard-link hole. It does not: a hard
link to a file owned by the daemon's *own* user still passes, because it *is*
owned by that user. Such a file cannot have been planted by a different local
user, so it is not the attack the check targets. The comment now says so, and
also records that the compared uid is the **effective** uid and that the
daemon performs no privilege drop anywhere in the tree, so the euid is stable
for the process lifetime.

**Found by the independent review** (observation 8).

---

## Reviewed and deliberately NOT changed

| Finding | Decision |
|---|---|
| `panic = "abort"` vs. the documented "otherwise continues" posture | **Kept.** Both `catch_unwind` sites are inside `#[cfg(test)]`, so no production code relies on unwinding. It is a production default; changing it needs the owner's decision. The doc mismatch stays on the open list. |
| IPC `BATCH_MAX` applied after serde allocates the batch | **Kept.** The allocation is already bounded by the 1 MiB frame limit, so this is a hardening gap, not an exhaustion vector. Changing the wire contract to reject instead of truncate is an API decision. |
| The default unauthenticated privileged IPC posture | **Not changed.** Policy decision for the owner. |
| Tracked stale copies `README.md.bak`, `src/engine/boot.rs.bak`, `src/engine/boot.rs.orig` | **Not changed.** On the preserve list; deletion needs an explicit owner decision. |
| The mesh findings (CRDT tombstone race, unblock-not-projected-but-ACKed, cursor regression, per-peer authorization) | **Not in this batch.** The owner asked for the non-mesh findings first. They are the next candidates. |

---

## Verification

All commands run in `/home/m/.cache/rs-master` with `CARGO_TARGET_DIR` unset
(the nested BPF build deadlocks when it inherits one) and
`PATH="$HOME/.local/bin:$PATH"` for `bpf-linker`.

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo check --workspace --locked --all-targets` | PASS |
| `cargo check --workspace --locked --all-targets --all-features` | PASS |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | PASS |
| `cargo clippy --workspace --locked --features full --all-targets -- -D warnings` | PASS |
| `cargo build --workspace --locked --all-targets` | PASS |
| `cargo build --workspace --locked --all-targets --all-features` | PASS |
| `cargo test --workspace --locked` | PASS |
| `cargo test --workspace --locked --features full` | PASS |
| `cargo test -p ramshield-mesh --locked` | PASS |
| `cargo test --workspace --locked --all-features` | PASS |
| `cargo build --release --locked --features full` | PASS |
| `lint-no-unwrap` (production `src/` + `crates/`) | PASS — 0 hits |
| `scripts/verify_shm_header.sh` | PASS |

Full gate log: `/tmp/f01/perf2_gates.log`.

---

## Review observations closed

The independent review of this batch (`docs/development/MASTER_SCAN_PERF_REVIEW.md`,
VERDICT: PASS) raised eight optional observations. Disposition:

| Obs | Disposition |
|---|---|
| OBS-1 — the buffer-based line check would reject pipelined valid frames | **Fixed before the push.** The check was reworked to measure only the untermined TAIL (the buffer can hold several complete pipelined frames, so its length is not the length of any one line), and to apply only when the line limit is the tighter of the two bounds. Covered by `ipc_wiring_unterminated_line_is_capped_at_the_tighter_line_limit`. |
| OBS-2 — the value semantics change | **Verified and documented** above. The kernel TTL, the keep-alive and `expire_due` all agree; the `PERMANENT` rewrite was redundant. |
| OBS-3 — the CIDR trie still re-inserts every expected CIDR | **Accepted.** An LPM trie is not a hash map; there is no "present" set to diff against, and re-inserting is the trie's contract. The quadratic *membership test* was the defect and is fixed. |
| OBS-4 — a peer's merge now blocks for the duration of a full-map walk instead of skipping | **Accepted, deliberately.** The old "skip" was what allowed the O(keys × map size) flush blowup. The trade is latency for a hard bound, and the bound is what keeps the detection path affordable under attack. |
| OBS-5 — the local path now fsyncs per block under `GroupCommit` | **Accepted, with a caveat recorded.** It is the same cost the mesh path already pays, and it is what makes "never ahead of the durable log" true for the local path too. A deferred-merge design (merge only once the record is durable, without forcing the window closed) would remove the cost and is the natural follow-up. |
| OBS-6 — the torn-tail log line | **Closed** — that is P-6 in this document. |
| OBS-7 — the CGNAT test leaves its temp file behind | **Accepted.** One small file per test process in the temp dir; the crate has no `tempfile` dev-dependency, so removing it would mean hand-rolling cleanup for no benefit. |
| OBS-8 — `status_dist` is `u32` while `count`/`bytes` are `u64` | **Accepted** as a pre-existing concern outside this scope. The saturating guard removes the panic path. |
