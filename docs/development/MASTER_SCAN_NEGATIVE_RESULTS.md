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

## 4. REFUTED (my own error) — the "release `panic = "abort"`" mismatch

**This item was wrong and is withdrawn.** It was carried in the master-scan
findings list as a documentation mismatch. Checked against the source:

- `Cargo.toml:108` `[profile.release]` sets only `opt-level`, `lto` and
  `codegen-units`. **There is no `panic = "abort"`.**
- `docs/DEFENSE_MODEL.md` contains no mention of panics, fail-fast, or
  fail-open at all.
- The only panic-related claim is `AGENTS.md:114` ("XDP panics only on
  catastrophic map corruption; otherwise continues") and `AGENTS.md:93` ("XDP
  decisions are fail-open"). The production XDP path has no panic site at all:
  every `unwrap`/`expect` in `crates/ramshield-xdp/src/lib.rs` is inside a
  `#[cfg(test)]` block, and the eBPF program cannot panic — a failed map
  update is a helper error that returns the fail-open XDP action.

**No change made.** The two AGENTS.md lines are loose wording, not a false
resilience claim, and AGENTS.md is a controlling document — it is not edited
for a wording nit.

---

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

### 7c. L7 (documentation) — the docs were accurate; the security model was not

`docs/CONFIGURATION.md:80` already said *"A non-loopback dashboard bind
requires `admin_password_hash`"*, and `:143` said *"public dashboard binds
require an admin password hash"*. Those statements are correct, so the
triage's "the fail-closed doc claim is only true for public binds" was a
statement about the code, not a documentation defect.

The real documentation defect was elsewhere — see the next section.

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

