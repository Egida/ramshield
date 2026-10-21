# Master scan — negative results, refutations, and parked findings

Date: 2026-10-21
Scope: the non-mesh master scan, plus the mesh findings the concurrency lens
raised. Every item below was checked against the source at `d41b286`
(`origin/master`) and is recorded so the same lead is not re-investigated.

These are results, not work items. Where a lead was **REFUTED**, the reason
is given so it can be closed with confidence. Where a lead was confirmed but
falls inside parked scope, the exact roadmap section is named so the parked
work starts from a verified input instead of a raw report.

---

## 1. REFUTED — a peer CANNOT suppress another node's ban

**Raised by:** the concurrency lens, as its third finding.
**Claim:** a peer that sends an unblock for an IP this node blocked locally
leaves the Store/XDP blocked while the CRDT holds a tombstone, and the sender's
cursor advances past that LSN, so the divergence is never repaired.

**Refuted.** The CRDT is origin-bound at the dot level:

- `crates/ramshield-mesh/src/aworset.rs:573` `merge_unblock_delta` keys the
  tombstone by `(delta.ip, delta.dot.node_id, delta.dot.incarnation)` and
  removes the entry under that same key. A peer's unblock therefore removes
  only **its own** dot.
- `crates/ramshield-mesh/src/aworset.rs:331` `would_be_blocked_after_unblock`
  scans every entry for the IP and excludes **only** the delta's own dot, so
  the node's own dot keeps the IP blocked.
- `crates/ramshield-mesh/src/aworset.rs:362` `merge_delta` applies the same
  origin-scoped tombstone check.

A local or operator block creates its own dot
(`crates/ramshield-enforcement/src/service/enforce.rs:362`, `mesh.record_ban`),
so it lives under a different key from any peer's dot and cannot be removed
by that peer.

**The branch the lens pointed at is also correct.** At
`crates/ramshield-enforcement/src/service/mesh.rs:660`, the condition
`changed && (still_blocked || !locally_applied)` takes the
journal-and-merge-only path exactly when the IP must NOT be locally unblocked
— either another live dot still blocks it, or this node never applied a mesh
block for it and a peer must not clear a local one. The `Ok(Ignored)` return
is honest: the record is durable and the CRDT is merged, which is what
`mesh_ackable` (`service/mesh.rs:717`) means by "accepted".

**No change made.**

---

## 2. PARKED (F-02) — an older block can override a durable removal

**Raised by:** the concurrency lens, as its first finding.
**Confirmed.** `merge_delta` (`crates/ramshield-mesh/src/aworset.rs:362`)
performs a check-then-act that is not atomic with respect to the tombstone
map: it reads the tombstone for the key, and only then inserts the entry. A
concurrent unblock that installs a newer tombstone for the same
`(ip, node_id, incarnation)` key between those two steps leaves the older
block entry present, so a removed ban re-appears.

**Why it is parked.** Roadmap v1.2 §3 (F-02) states the requirement verbatim:
*"Prevent an older block from overriding a durable removal"* and, in
Acceptance, *"No committed-unblock resurrection"*. F-02 also requires a
mandatory design note reviewed before coding, and the roadmap sequences F-02
only after F-01's independent review. The standing instruction is *"No F-02
work may begin."*

**Recorded for F-02:** the concrete interleaving is (1) block delta for
`(ip, node, incarnation, counter=5)` and (2) unblock delta for the same key
with `counter=6` are merged concurrently; the block path reads the tombstone
map (empty), the unblock path installs tombstone(6), and the block path then
inserts entry(5) — which now exists despite a newer tombstone. The fix must
make the tombstone check and the entry insert atomic for the key, and must
preserve the "older block never overrides a durable removal" invariant
under concurrent merges.

**No change made.**

---

## 3. PARKED (F-03) — the unblock acknowledgement and cursor path

**Raised by:** the concurrency lens, as its second finding.
**Confirmed as described, and scoped to F-03.** The mesh acknowledgement
contract is "durably accepted", not "projected": `mesh_ackable`
(`crates/ramshield-enforcement/src/service/mesh.rs:717`) returns `true` for
`Ok(Ignored)` and for a post-durability storage/XDP failure. The sender then
advances its per-peer WAL cursor past that LSN. Projection failures are
repaired by `mesh_projection_pending` and the reconciler
(`service/mesh.rs:336`), but the journal-and-merge-only branch at
`service/mesh.rs:661` does not populate that set — correctly, because in
that branch the IP must not be locally unblocked (see §1).

**Why it is parked.** Roadmap v1.2 §4 (F-03) is "WAL replication continuity
and cursor safety", and its scope covers exactly this: the LSN semantics of an
acknowledged-but-not-projected mutation and the cursor advancement that
follows. F-03 must define the LSN semantics before the code changes.

**No change made.**

---

## 4. CORRECTION — the release profile DOES set `panic = "abort"`

**An earlier version of this document withdrew this finding as my own
error. That withdrawal was itself wrong, and the withdrawal is hereby
corrected.** The `079cbae` commit message carries the same error.

What happened: I read `[profile.release]` from `Cargo.toml` and concluded it
set only `opt-level`, `lto` and `codegen-units`. The profile actually reads:

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"        # Cargo.toml:112
strip = true
overflow-checks = true
```

I read past the `panic = "abort"` line. The finding is **real**, and the
documentation is what is wrong.

### The verified facts

- `Cargo.toml:112` — the release profile sets `panic = "abort"`, and
  `overflow-checks = true`. **Any panic in any thread aborts the entire
  daemon process.** There is no per-thread survival and no unwinding.
- `AGENTS.md` (Error Handling & Resilience) claimed *"Fail-fast: XDP panics
  only on catastrophic map corruption; otherwise continues"*. The
  "otherwise continues" is false as a general property of the daemon under
  `panic = "abort"`.
- `AGENTS.md` (eBPF/XDP Safety) also claims *"XDP decisions are fail-open
  (map update failure → continue without blocking)"*. **That claim is
  correct** and is left alone: it describes a BPF map update failure, which
  is a helper error returning the fail-open XDP action — not a panic. The
  production XDP path genuinely has no panic site: every `unwrap`/`expect`
  in `crates/ramshield-xdp/src/lib.rs` is inside a `#[cfg(test)]` block, and
  the eBPF program cannot panic.
- The code comments that reference `panic = "abort"` are **correct** and
  were right all along: `crates/ramshield-detection/src/batch.rs:39,51`,
  `crates/ramshield-detection/src/tests/mod.rs:1411` and
  `crates/ramshield-protocol/src/auth.rs:158`. This is exactly why the
  saturating-counter fixes in this series matter: with `overflow-checks = true`
  an unguarded `+=` on a client-supplied value is a panic, and under
  `panic = "abort"` that is a process abort, not a caught error.
- `docs/DEFENSE_MODEL.md` contains no mention of panics, fail-fast or
  fail-open, so it makes no claim to correct.
- `docs/ENGINEERING_CONSTITUTION.md:97` claims *"No production constructor
  panics because an external resource is missing. No production
  `unimplemented!()`, `todo!()`, or unproven `unwrap()`."* **That claim is
  true** and is enforced by the `lint-no-unwrap` gate (0 hits on production
  `src/` and `crates/`). It is left alone.

### The fix

`AGENTS.md` now states the real property: the release profile sets
`panic = "abort"`, so a panic in any thread aborts the whole daemon, and the
reason the XDP path is nonetheless panic-free is that it has no panic site —
not that the daemon survives one.

## 5. REFUTED — `deployment_cli.rs` unwraps outside the lint's scope

**Claim:** the no-unwrap lint scans only `src/` and `crates/`, so the
root-level `deployment_cli.rs` escapes it with real `unwrap()` calls.

**Refuted.** `deployment_cli.rs` is a `#!/usr/bin/env rust-script` deployment
helper, not a workspace member. It has exactly two textual matches for
`unwrap`, and both are **string literals**: `deployment_cli.rs:330` is the
pattern `".unwrap()"` used by its own malicious-code check, and
`deployment_cli.rs:376` is the description of a pattern to look for. There
are zero actual `unwrap()`/`expect()` calls in the file.

**No change made.**

---

## 6. NO CHANGE — the replay store is sound against replay

**Claim (open scan lead):** the replay store's bounds might let a replayed
message be accepted again, or grow without limit.

**Verified sound.** `crates/ramshield-protocol/src/auth/replay_store.rs`:

- The store is bounded: a global cap, a per-key cap, and a TTL, with
  saturation **rejecting** new nonces rather than evicting live markers
  (`replay_store.rs:107`), so capacity pressure cannot reopen a replay
  window.
- The TTL sweep (`replay_store.rs:84`) only ever removes a marker whose
  *current* map timestamp is expired, so a live marker is never dropped. The
  map — not the queue — is the authority for the replay check
  (`replay_store.rs:95`).
- A captured frame cannot outlive its replay marker:
  `MAX_CLOCK_SKEW_MS` is 10 000 ms (`crates/ramshield-protocol/src/auth.rs:24`)
  while the production replay TTL is 65 s (`src/ipc/server.rs:206`). The
  marker always outlives the clock validity window, so once the marker
  expires the frame is already rejected by the timestamp check.

**Residual, P2, not fixed:** `check_and_record` can push a second copy of the
same key onto the `order` queue while the map holds one entry, so the queue
can hold duplicates. The consequences are (a) the per-key counter at
`replay_store.rs:102` counts queue entries rather than distinct keys, so the
per-key cap is consumed early, and (b) the queue is bounded by
`cap x per_key_cap` rather than by `cap`, giving a bounded memory
amplification of roughly 25 MB at the production values (global cap 1024,
per-key cap 256, TTL 65 s). Reaching it requires re-sending the same nonce
after its TTL, which — per the clock-window argument above — is not reachable
by replaying a captured frame, and requires an attacker who already holds a
valid key and could saturate the store far more cheaply by sending distinct
nonces. There is **no security impact** and the bound is finite, so the
anti-replay path is left unchanged.

---

## 7. The remaining non-mesh lead triage — resolved

The triage is recorded in `docs/development/NONMESH_LEADS_TRIAGE.md`. Three
of its seven leads came back PARTIAL; all three were re-checked directly here
and all three resolve to REFUTED.

### 7a. L2 — the native TPACKET_V3 frame parser is sound

The triage could not fully verify the `tp_len` / `tp_snaplen` relationship.
Checked directly in `src/engine/native.rs`:

- `tp_len` is **never read** — it is declared in the header struct
  (`native.rs:85`) and used nowhere. The packet slice is sized by
  `tp_snaplen`, which is the captured length, not the original length.
- `tp_snaplen` and `tp_mac` are both required to be `<= frame_size` (2048)
  before use (`native.rs:479`).
- `blk_len` is validated by `valid_tpacket_block_len`
  (`native.rs:544`): it must be at least `size_of::<TpacketBlockDesc>()` and at
  most the block size.
- Every offset is computed with checked arithmetic and range-checked against
  **all three** bounds — the block, the block descriptor length, and the
  mapped ring. `checked_tpacket_header_offset` (`native.rs:555`) and
  `checked_tpacket_packet_offsets` (`native.rs:579`) both do this, and the
  packet slice is only taken once
  `packet_abs_end <= block_end && packet_rel_end <= blk_len` holds.
- `checked_next_tpacket_offset` (`native.rs:601`) rejects a `tp_next_offset`
  that does not advance past the current header, and uses checked alignment
  arithmetic.

**No change made.**

### 7b. L7 — the dashboard's unauthenticated default is a tested decision

The triage reported the dashboard as failing open: enabled by default, no
`admin_password_hash` by default, and `auth.enabled()` is
`password_hash.is_some()` (`src/dashboard/auth.rs:160`), so with no hash
`require_auth` passes every request through.

That is accurate, but it is **not an oversight** — it is a tested product
decision. `crates/ramshield-config/src/tests/mod.rs:272`
(`default_config_validates`) asserts that the default config produces **no**
exposure warnings, with the message *"loopback-only config must warn about
nothing"*. The unauthenticated loopback posture is deliberate, and the
escalation path is closed: `Config::validate()` rejects a public dashboard bind
without a password hash (`crates/ramshield-config/src/validate.rs:368`) and
without TLS (`:375`), and `api_set_config` re-validates the merged config
before applying it. The dashboard exposes no unblock route — its routes are
read-only plus the validated `POST /api/config` (`src/dashboard/mod.rs:52-63`).

**Adding a startup warning for this case was considered and rejected**: it
would have required changing the existing `default_config_validates`
assertion, and that test encodes the decision. The existing test was left
untouched.

### 7g. The kernel-side block maps under extreme traffic — sound

The requirement is that the RAM-side and kernel-side maps of blocked IPs stay
correct and bounded under extreme high traffic. Every layer was checked.

**1. The per-IP kernel map is LRU — a flood cannot fill it.**
`crates/ramshield-xdp/ramshield-xdp-bpf/src/main.rs:27` declares
`BLOCKLIST`/`BLOCKLIST6` as `LruHashMap`, with the comment *"LRU_HASH: kernel
auto-evicts coldest entry when full — no E2BIG under botnet flood."* The
kernel evicts the coldest entry rather than failing the insert, so a flood of
unique source IPs degrades the oldest blocks instead of permanently breaking
the map.

**2. The per-IP userspace map is capped, atomically, and fails closed.**
See §7f: `write.rs:108-129` reserves capacity before the entry exists with a
CAS loop, and rejects with `RsError::CapacityExceeded` plus a `warn!` when
the limit would be exceeded.

**3. The CIDR LPM trie has no LRU and no TTL in its value — and is still
bounded.** `BLOCKCIDR`/`BLOCKCIDR6` are `LpmTrie<[u64; 2], u8>` with 102,400
entries (`main.rs:37`). The kernel offers no eviction for an LPM trie, and
the `u8` value carries no expiry, so the BPF lookup
(`BLOCKCIDR6.get(&Key::new(128, key)).is_some()`, `main.rs:186`) enforces a
prefix until it is explicitly removed. Two facts make that bounded:

- The reconcile removes **every** trie entry that is not in the expected set
  (`crates/ramshield-enforcement/src/xdp.rs:668-679`), so the trie's contents
  are exactly the expected set, not a monotone accumulation.
- The expected set is `store.active_cidrs`
  (`crates/ramshield-enforcement/src/service/run.rs:7-8`), and CIDR expiry
  removes the network from it
  (`crates/ramshield-enforcement/src/service/enforce.rs:569-573`), so an
  expired prefix leaves the expected set and is removed from the trie on the
  next reconcile (every 10 s: 40 ticks x 250 ms, `service/run.rs:34-39`).

**The residual is a bounded over-enforcement window of at most one reconcile
interval (10 s)**, because the kernel-side entry has no TTL of its own. That
is the safe direction for a mitigation system.

**4. CIDR blocks are operator-driven, so the "tens, not thousands" assumption
holds.** `BlockCidr` originates only from the IPC `block_cidr` request
(`src/ipc/server/handlers.rs:85`) and the enforcement command path
(`service/enforce.rs:121,325`). The detection engine's subnet leg
(`crates/ramshield-detection/src/subnet_scan.rs`, `flush.rs:183`) computes
subnet *counts* and promotes individual IPs; it never issues a CIDR block. A
botnet flood of source IPs therefore produces per-IP blocks, which are
LRU-bounded, and cannot fill the 102,400-entry prefix trie.

**5. The userspace CIDR membership check is a linear scan — deliberately.**
`crates/ramshield-storage/src/store/access.rs:17-27` documents it: *"the set
is bounded by subnet decisions (tens, not thousands). Upgrade to an LPM trie
if a deployment ever holds thousands of concurrent prefix blocks."* Given
point 4, that assumption is sound, and the upgrade path is recorded.

**No change made.** The kernel and userspace block maps are bounded at every
layer, the LRU absorbs per-IP floods, the prefix trie is self-cleaning, and
the one residual — a 10 s over-enforcement window on an expired prefix — is
bounded and in the safe direction.

---

### 7f. The store's memory bound under extreme unique-IP traffic — sound

A flood of distinct source IPs is the obvious way to make a per-IP state map
run out of memory, so the store's bound was checked directly.

**It is bounded, atomically, and it fails closed.**

- `crates/ramshield-storage/src/store/write.rs:108-129` — the fresh-insert
  path (`dashmap::Entry::Vacant`) reserves capacity **before** the entry
  exists, with a CAS loop so two threads cannot both observe "fits" and both
  insert. When the reservation would exceed the limit the insert is
  **rejected** with `RsError::CapacityExceeded` and a `warn!` is logged
  (*"store insert rejected: capacity exceeded"*). The tracking stops; it
  does not grow.
- The limit comes from `engine.ram_limit_mb`
  (`crates/ramshield-config/src/sections/engine.rs:16`), default **512 MB**.
- `crates/ramshield-storage/src/store/write.rs:35-58` — the update path
  (`dashmap::Entry::Occupied`) deliberately does **not** re-check the limit;
  the comment states that *"the limit only gates net-new growth"*. The
  over-shoot that permits is bounded: the dominant per-IP value,
  `IpRecord` (`crates/ramshield-storage/src/record.rs:129`), is a pure-value
  struct with **zero heap side-allocation** — `heap_bytes()` returns 0 for
  it (`record.rs:117`) and the comment records that returning
  `size_of::<IpRecord>()` there previously double-counted ~136 B per blocked
  IP and starved the budget to half its real size. So an IP entry's size is
  fixed, and an update cannot grow it.
- `crates/ramshield-storage/src/store/capacity.rs:58-66` — `evict_expired`
  returns 0 immediately when nothing carries a TTL, which is the production
  case (store inserts pass no TTL; block TTLs live in the enforcement actor).
  That is why the bound is enforced at insert time rather than by sweeping.

**No change made.** The RAM-side map of blocked IPs is hard-bounded by
configuration, and exceeding the bound fails closed with a warning rather
than growing the process — which is the property the extreme-traffic
requirement asks for.

---

### 7e. The triage's "recommended next fixes" — two accepted, three rejected

The triage ended with six recommendations. Checked against the source:

- **L2 — REJECTED, and applying it would introduce a bug.** The suggestion
  was to bound the native copy by `min(tp_snaplen, tp_len)` and to add
  `tp_len <= frame_size`. `tp_len` is the **original** packet length, before
  capture truncation, so it can legitimately exceed the snaplen and exceed
  `frame_size`; rejecting on `tp_len > frame_size` would reject valid
  frames. And `tp_len` is never read at all — the packet slice is sized by
  `tp_snaplen`, the captured length, which is the correct value to copy.
  `src/engine/native.rs` never references `tp_len` outside the header struct
  definition. No change.
- **L6 — REJECTED, on a premise that was stale.** The suggestion was to
  increment `blocks_total` only when the XDP apply succeeded. The metric's
  exported `HELP` text is *"Total blocks issued."* — which is exactly what it
  counts — and the failures are exported separately as
  `ramshield_xdp_apply_failures_total`; the count of blocks actually in
  effect is `snapshot.blocks_applied`, which the dashboard already serves. The
  three counters are coherent and each is named for what it measures. The
  recommendation rests on the metrics README, which said *"Total block
  decisions applied"* — that was a **stale README**, corrected in `079cbae`;
  the code was never wrong. An earlier note of mine that the smoke script
  would break under the change was also wrong:
  `scripts/metrics_smoke.py:180` reads the SSE field
  `detection.blocks_total`, which is `snapshot.blocks_applied`
  (`src/dashboard/mod.rs:454`) — a different counter from the Prometheus
  metric. No change.
- **L4 — ACCEPTED as a documentation fix.** `POST /api/config` applies the
  patch to the **running engine's** config handle
  (`state.engine.config.store(..)`, `src/dashboard/mod.rs:399`) and nothing in
  the dashboard path writes to disk, so **a patched configuration is reverted
  on restart**. That is now stated in `docs/OPERATIONS.md`, together with the
  CSRF behaviour (fails open when neither `Origin` nor `Referer` is present)
  and the authentication consequence.
- **L3 — NOT ACTIONED.** The dashboard's body-reading routes are already
  bounded by axum's default extractor limit (2 MiB), and the dashboard binds
  loopback by default. Tightening it to 64 KiB would be a default change that
  could reject a legitimate large configuration patch, for a local-only
  surface. Raised with the owner as part of Decision 1 rather than changed
  unilaterally.
- **L5 — NOT ACTIONED.** Argon2 is constant-time, the failure table is
  bounded, the CPU is semaphore-bounded, and every error path fails closed. A
  global (rather than per-IP) login-failure ceiling is a policy choice, not a
  defect.
- **L7 — POLICY, folded into Decision 1.** See the owner decision package.

---

### 7c. L7 (documentation) — the docs were accurate; the security model was not

`docs/CONFIGURATION.md:80` already said *"A non-loopback dashboard bind
requires `admin_password_hash`"*, and `:143` said *"public dashboard binds
require an admin password hash"*. Those statements are correct, so the
triage's "the fail-closed doc claim is only true for public binds" was a
statement about the code, not a documentation defect.

The real documentation defect was elsewhere — see the next section.

---

## 7d. The second reviewer's observations — all resolved

The independent review of the performance/durability batch
(`docs/development/MASTER_SCAN_PERF_REVIEW.md`) raised eight optional
observations. Each was re-checked against the source at `009ab24`:

- **OBS-1 (IPC length check) — already fixed.** The reviewer read the
  *intermediate* implementation, which bounded the whole buffer. The current
  check at `src/ipc/server/connection.rs:93` measures the newline's position
  in the buffer — the length of that one line — so pipelined valid lines are
  not rejected. Covered by
  `ipc_wiring_unterminated_line_is_capped_at_the_tighter_line_limit`.
- **OBS-2 (XDP map values / TTL) — already closed.** `crates/ramshield-enforcement/src/xdp.rs:610-625`
  documents the semantics: a resident key keeps the expiry `apply_block` gave
  it; the keep-alive re-runs the enforce path at `ttl/2` and so refreshes the
  kernel expiry before it passes; the only difference is that a *missed*
  keep-alive now lets the TTL lapse instead of turning the block
  `PERMANENT`. The reviewer could not read the BPF expiry check within
  budget; the eBPF program compares `now < expiry` against
  `bpf_ktime_get_ns`, the same monotonic clock.
- **OBS-3 (LPM trie re-insert) — accepted residual.** The LPM trie API
  offers no "present key" diff, so every expected CIDR is still re-inserted
  each cycle. Documented.
- **OBS-4 (deadlock in `flush_pre_aggs_locked` → `flush_batch`) — no
  deadlock.** The merge lock is taken in exactly two places,
  `crates/ramshield-detection/src/pre_aggs.rs:28` (`merge_local`) and `:94`
  (`flush_pre_aggs_to_store`); the only callers of those two are
  `crates/ramshield-detection/src/workers.rs:134/152/178`, **sequential and
  never nested**, and `flush_batch` is not among them. Empirically, the
  pressure path is exercised by the existing test
  `local_merge_enforces_shared_pre_aggregation_capacity`
  (`pre_aggs_max_size = 2`, five distinct IPs), which forces
  `merge_local` → `flush_pre_aggs_locked` → `flush_batch` and passes in
  0.02 s — a re-entrant `std::sync::Mutex` would hang it and every CI run.
  `merge_local_holds_the_merge_lock_for_the_whole_drain` also passes.
- **OBS-5 (WAL fsync cost / mesh double-sync) — no double-sync.** The mesh
  path is a separate function, `append_mesh_wal`
  (`crates/ramshield-enforcement/src/service/enforce.rs:181`), with its own
  `wal.sync()`; the local path's `append_is_durable()` check is in a
  different function, so nothing syncs twice. The shipped default WAL
  durability is `Durability::Flush`
  (`crates/ramshield-config/src/sections/wal.rs:29`), which is not
  `SyncEach`, so `append_is_durable()` is false and the local path does
  fsync per acknowledged block — the documented trade-off of the
  durable-before-merge fix.
- **OBS-6 (torn-tail reporting) — already fixed** by
  `log_unrepaired_tail`; the boot recovery path is untouched.
- **OBS-7 (cgnat temp file cleanup) — the tests clean up.** Each
  `crates/ramshield-cgnat/src/shm.rs` test ends with
  `let _ = std::fs::remove_file(path);`.
- **OBS-8 (`status_dist` 255 marker) — not a defect.** `255` is a "not in a
  known class" sentinel in the `STATUS_BUCKET` lookup table
  (`crates/ramshield-detection/src/batch.rs:44`), guarded by `if b != 255`
  before the index is used. It is not a counter value. The counters
  themselves are saturating.

---

## 8. Documentation accuracy — the IPC trust boundary

Three documents described the authenticated IPC without saying that
authentication is **off by default**.

**`docs/SECURITY_MODEL.md:14`** labelled the IPC hop in the trust-boundary
diagram as an `authenticated boundary`. With the default configuration
(`ipc.auth_keys` empty, `ipc.require_auth` false) the IPC applies no
authentication and no role check at all, and binds `127.0.0.1:7890`. The
diagram now states the condition, and a paragraph below it says plainly that
the boundary is authenticated only when `ipc.auth_keys` is configured, that
the default is loopback-only and unauthenticated, and that a non-loopback bind
fails `validate()` unless HMAC keys are set and a TLS-terminating proxy is
asserted.

**`docs/IPC.md`** scoped the authentication envelope correctly (*"When
`ipc.auth_keys` is non-empty"*) but then presented the identity guarantee
(*"no hard-coded `admin`"*), the authorization guarantees, and a **Min role**
column for every request as if they always applied. A new *Default posture*
subsection now states the actual default — unauthenticated, no role check, the
historical hard-coded `"admin"` actor, loopback-only — and the rest of the
section is explicitly labelled *Authenticated mode*. The Min role column
carries a note that it applies only when a principal is present.

**`crates/ramshield-metrics/README.md`** documented `ramshield_blocks_total`
as *"Total block decisions applied"*, but the counter counts decisions
**issued** — the exported `HELP` text says *"Total blocks issued."*, and a
block whose kernel projection failed is still counted. The failure count is
already exported separately as `ramshield_xdp_apply_failures_total`, so the
README now says which counter is which and gives the applied-blocks
expression. The counter's behaviour is unchanged.

**No behaviour changed in this batch.**

