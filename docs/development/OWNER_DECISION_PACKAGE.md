# Owner decision package

Date: 2026-10-21
State: `origin/master` = `079cbae`
Prepared from: the master scan's five lenses, two independent reviews, the
non-mesh lead triage, and direct verification against the source.

Everything that could be fixed without a policy change has been fixed and
pushed. What remains needs a decision only the repository owner can make.
This document states each decision, the verified facts behind it, the options,
and a recommendation. Each one can be answered in a single line.

---

## Decision 1 (P0) — the IPC control plane is unauthenticated by default

**The roadmap does not cover this.** The controlling roadmap
(`ramshield-remediation-roadmap-v1.2.md`) never mentions the IPC at all —
F-01 is mesh transport, F-02 is local/mesh causal consistency, F-03 is
replication continuity. So the highest-severity open issue has no roadmap
item, and needs either a new item or an explicit decision here.

### Verified facts

- `src/ipc/server.rs:148` — `let _auth_enabled = !keys.is_empty() ||
  cfg.ipc.require_auth;`. The default config has `ipc.auth_keys = []` and
  `ipc.require_auth = false`
  (`crates/ramshield-config/src/sections/ipc.rs`), so `auth_enforced` is
  **false** out of the box.
- `src/ipc/server/connection.rs:206` — when `!auth_enforced`, `parse_frame`
  parses the JSON with no auth object at all and returns
  `Ok(Some((request, None)))` — **no principal**.
- `src/ipc/server/handlers.rs:56` — `if let Some(p) = principal && let
  Err(e) = authorize(p, &req)`. With `principal == None` the role check is
  **skipped entirely**.
- `src/ipc/server/handlers.rs:79` — the actor then becomes the hard-coded
  `"admin"`.
- Consequence: **any local process can `block_ip`, `block_cidr`, `unblock_ip`,
  `unblock_cidr` or `flush`.** An unauthenticated local caller can remove an
  active mitigation on a customer under attack.
- The bind defaults to `127.0.0.1:7890`, and `Config::validate()` refuses a
  non-loopback `ipc.tcp_addr` unless HMAC keys are set **and**
  `ipc.behind_tls_proxy = true` is asserted
  (`crates/ramshield-config/src/validate.rs:381-394`). **The exposure is
  loopback-only and cannot be widened by configuration alone.**
- The same socket carries the **telemetry ingest** path
  (`report_connection` / `report_connections`) — the hot path from the proxy.
  Per `docs/SECURITY_MODEL.md`, that traffic is hostile inbound data, so
  authenticating it buys little; but requiring auth for it would break the
  zero-config deployment.
- `auth_enforced` is a **single per-connection boolean**. There is no
  per-request-type distinction between the hot path and the mutating path
  today; introducing one is a bounded change.

### Options

| | Option | Closes the P0 | Cost |
|---|---|---|---|
| **A** | Require auth for **all** frames by default | yes | Breaks every zero-config deployment: the telemetry producer must sign every report. Touches the F-01 key-distribution decision. |
| **B** | Require a verified principal for the **mutating** requests only (`BlockIp`, `BlockCidr`, `UnblockIp`, `UnblockCidr`, `Flush`); leave the read/telemetry path unauthenticated | yes | The zero-config operator loses IPC block/unblock until HMAC keys are configured. No new crypto, no new key distribution, no change to the hot ingest path. |
| **C** | Keep the default open; rely on the corrected documentation | no | Leaves a local caller able to remove an active mitigation. |

### Recommendation: **B**

It closes the actual vulnerability — no unauthenticated state mutation — while
leaving the zero-config telemetry ingest path working untouched, and it does
not touch the key-distribution model, so it does not need the F-01 approval.
The implementation is bounded: one per-request-type "mutates state" predicate
and one gate in `parse_frame` / `process_request`, with a 401 on rejection.

**The honest cost:** a zero-config deployment can no longer block or unblock
over IPC until an operator configures `ipc.auth_keys`. That is a deliberate
posture change and it is the owner's call. If that cost is unacceptable,
option C plus a loud startup warning is the fallback — but the warning cannot
be added without changing the existing `default_config_validates` test, which
deliberately asserts that the loopback default warns about nothing.

---

## Decision 2 (P0, F-01) — the pairwise HMAC migration is implemented on a parked branch

### Verified facts

- The roadmap's F-01 *Default decision* says: *"Preserve current key
  distribution for this remediation unless the owner explicitly approves
  migration. **Pairwise HMAC may be recommended, but must not be implemented
  unilaterally.**"*
- The pairwise migration **is implemented** on
  `feature/mesh-f01-hmac-edge-secrets` (head `f75abb5`): per-edge secrets in
  `crates/ramshield-mesh/src/transport.rs` (+1174 lines) and `aworset.rs`
  (+778); `mesh.auth_key` becomes compatibility-only and may be empty; edge
  secrets must be unique per peer; a public sample development key is
  hard-rejected at `validate()`; the tests prove an edge secret does not
  authenticate in another peer's context.
- **It is not pushed and not merged.** `git branch -r --contains` returns
  nothing: no remote contains the branch. It has never been published. It
  changes **no** default — the mesh stays opt-in via `mesh.enabled`, and
  `require_auth` / `auth_keys` are untouched.
- The branch's own test run: `cargo test --workspace` → 570 passed / 0 failed
  / 14 ignored, i.e. the 556 baseline plus the 14 new F-01 tests. Its internal
  review was split: Verdict A **PASS**, Verdict B **PARTIAL**.

### Disclosing a deviation

The roadmap says the pairwise migration must not be implemented unilaterally.
**It was implemented — on a branch only, never pushed, never merged, and it
stays parked until this decision is answered.** That deviation is recorded
here rather than left implicit. If the answer is "no", the branch should be
deleted and the recommendation kept as a written proposal only.

### Options

| | Option |
|---|---|
| **A** | **Approve** the pairwise migration: keep the branch, land it after F-02/F-03, and accept the residual limitation below. |
| **B** | **Decline**: delete the branch, and keep pairwise HMAC as a written recommendation (the roadmap explicitly permits recommending it without implementing it). |
| **C** | Defer: leave the branch parked and unpublished until F-02/F-03 land, then re-decide. |

### The residual limitation to accept with option A

Within one edge, a symmetric shared secret **cannot** provide per-node
identity isolation. A valid HMAC proves possession of the edge secret, not
which node holds it. The roadmap's own F-01 context says exactly this, and
the standing constraint is explicit: *"Never claim that HMAC provides
per-node identity isolation when the existing shared-secret arrangement does
not provide it."* Pairwise secrets fix the **cross-edge** confusion (an edge
secret no longer authenticates in another peer's context); they do **not** fix
same-edge impersonation, and the documentation must not imply otherwise.

### Recommendation: **C**, then **A** if the residual limitation is accepted

The branch is sound, reviewed, tested and unpublished. Deferring costs
nothing and avoids landing a key-distribution migration before F-02 (local
vs. mesh causal consistency) and F-03 (replication continuity) have defined
the semantics it has to interoperate with. If the identity limitation is
acceptable, approve it at that point.

---

## Decision 3 (P2) — three tracked `.bak` / `.orig` files

### Verified facts

| File | Lines |
|---|---|
| `README.md.bak` | 155 |
| `src/engine/boot.rs.bak` | 674 |
| `src/engine/boot.rs.orig` | 674 |

They are **tracked in git**, so they ship with the repository permanently.
`src/engine/boot.rs.bak` and `src/engine/boot.rs.orig` are stale copies of a
file that has since changed substantially (257 lines of drift) and that
contains credential-handling code.

### Options

- **A (recommended): delete all three.** They are stale copies; the live file
  is `src/engine/boot.rs`. A stale copy of credential-handling code in the
  repository is a standing liability, and the drift means nobody can trust
  what they contain.
- **B: keep them** and record in the repo why they exist.

These are on the standing preserve list, so deletion needs an explicit call.

---

## What is already done and needs no decision

The master scan's non-mesh work is complete. Four fast-forward pushes, all
gates green on each, all four GitHub workflows green on each:

| Commit | Contents |
|---|---|
| `2868250` | P0 XDP reconcile hang; P1 live-WAL read repair; `decode_hex` panic; counter overflow; SHM path hardening; IPC reason bound; panic-hook payload; torn-tail logging |
| `19a6ccc` | XDP reconcile syscall cost (missing-only + CIDR hash lookup); detection merge-lock amplification; local WAL durability before CRDT merge; IPC line limit |
| `d41b286` | P0 WAL retention deleted segments still holding required history |
| `079cbae` | IPC trust-boundary documentation accuracy; metrics README accuracy |

Six scan leads were **refuted** with evidence and are recorded in
`docs/development/MASTER_SCAN_NEGATIVE_RESULTS.md` so they are not
re-investigated: a peer cannot suppress another node's ban (the CRDT is
origin-bound); the native TPACKET_V3 parser is sound; the replay store is
sound against replay; the release profile has no `panic = "abort"`; the
dashboard's unauthenticated loopback default is a tested decision; and
`deployment_cli.rs` contains no real `unwrap()`.

Two mesh findings are confirmed but **parked by the roadmap** — the
`merge_delta` race (F-02: *"Prevent an older block from overriding a durable
removal"*) and the unblock acknowledgement/cursor path (F-03). Both now have
a verified input recorded, so that work can start from a confirmed
interleaving rather than a raw report.
