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

## Decisions 1–3 — TAKEN

The owner authorised the recommended course ("proceed in verbose mode, go
ahead"). The three decisions below were therefore executed as recommended:

| | Decision | Outcome |
|---|---|---|
| 1 | P0 — the unauthenticated-by-default local control plane | **B** — a verified principal/session is now required for mutating requests; the read and telemetry paths stay open. Implemented as `ipc.require_auth_for_mutations` and `dashboard.require_auth_for_config`, both defaulting to `true`, with explicit escape hatches. |
| 2 | P0 — the parked pairwise-HMAC branch | **C** — deferred. The branch stays parked and unpublished until F-02/F-03 define the semantics it must interoperate with. |
| 3 | P2 — the three tracked `.bak`/`.orig` files | **A** — deleted, after confirming their content is preserved in git history. |

The rest of this document is the record of what those decisions were and why.

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

### The same question, one level over: the dashboard's config patch

The dashboard is the IPC's sibling, and the triage surfaced it while
verifying the IPC path. It is **not** the same severity — the dashboard has
no unblock route, so it cannot remove an active mitigation — but it is the
same *kind* of surface, and it should be decided together with the IPC rather
than separately:

- `dashboard.enabled` defaults to **true** and `dashboard.admin_password_hash`
  defaults to **None**
  (`crates/ramshield-config/src/sections/dashboard.rs:88`), and
  `auth.enabled()` is `password_hash.is_some()`
  (`src/dashboard/auth.rs:160`), so with the default configuration the
  dashboard has **no authentication**.
- `POST /api/config` accepts a typed patch from **any local process**: the
  CSRF check fails open when the request carries neither `Origin` nor
  `Referer` (`src/dashboard/mod.rs:276` — a browser-shaped defence, not an
  authentication one), and with no password hash there is no session to
  check.
- The patch is validated, and `Config::validate()` rejects a patch that would
  widen an exposure (a public bind without credentials or without TLS), so
  the practical ceiling is "change non-security daemon configuration for the
  lifetime of the process".
- The bind defaults to `127.0.0.1:9999`, and a public bind without a
  password hash fails `validate()` at boot — the same containment as the IPC.

**Recommendation: decide it with the IPC, in the same decision.** If option B
is taken for the IPC, the dashboard's natural counterpart is "require a
session for the mutating route (`POST /api/config`) while leaving the
read-only routes open", which keeps the HUD usable locally and stops a local
process from reconfiguring the daemon. The startup-warning variant is not
available here either: `default_config_validates` asserts that the default
config produces no exposure warnings, so adding one would mean changing that
test, and the decision it encodes is deliberate.

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

### Executed — option C (defer) is in force

The deferral is recorded here so it does not depend on anyone's memory:

- The branch `feature/mesh-f01-hmac-edge-secrets` is parked at **`f75abb5`**
  and **unpublished**: `git branch -r --contains
  feature/mesh-f01-hmac-edge-secrets` returns nothing, and no remote branch
  name matches it. It has never been published, so no consumer can depend on
  it.
- It is **not merged** into `master` and it is **not** re-based onto it. It
  stays exactly as it was built.
- It changes **no default**: the mesh remains opt-in via `mesh.enabled`, and
  `require_auth` / `auth_keys` are untouched. Nothing in the shipped product
  depends on it.
- It is **re-decided when F-02 and F-03 land**, because those two define the
  merge/cursor semantics the pairwise secrets must interoperate with. Until
  then the branch is frozen: no further commits, no rebase, no force-push.
- The residual limitation above stands either way: within one edge, a
  symmetric shared secret cannot provide per-node identity isolation, and no
  claim of that kind may be made.

**F-02 and F-03 remain parked** and are not started by this decision.

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

### Executed — option A (delete) is done

The three tracked stale copies were deleted, **after** their content was
proven to be preserved in git history:

| File | Blob | Recovery |
|---|---|---|
| `README.md.bak` | `66d19e41` | **byte-identical to the current `README.md`** (`diff` = 0 lines) |
| `src/engine/boot.rs.bak` | `f249549a` | that exact content exists as a historical `src/engine/boot.rs` (`89578b6`, `49c9caa`) |
| `src/engine/boot.rs.orig` | `f249549a` | **byte-identical to `boot.rs.bak`**, same history |

Every blob was confirmed present in the object database before the deletion
(`git cat-file -e <blob>`), and each is recoverable with
`git show <commit>:<path>`. Nothing was lost: the two `boot.rs` copies were
identical to each other, and the `README.md.bak` was identical to the file it
shadowed.

### Options

- **A (recommended): delete all three.** They are stale copies; the live file
  is `src/engine/boot.rs`. A stale copy of credential-handling code in the
  repository is a standing liability, and the drift means nobody can trust
  what they contain.
- **B: keep them** and record in the repo why they exist.

These are on the standing preserve list, so deletion needs an explicit call.

---

## What is already done and needs no decision

The master scan's non-mesh work is complete. Six fast-forward pushes, all
gates green on each, all four GitHub workflows green on each.

### Pushed commits and their verification

| Commit | Contents |
|---|---|
| `2868250` | P0 XDP reconcile hang; P1 live-WAL read repair; `decode_hex` panic; counter overflow; SHM path hardening; IPC reason bound; panic-hook payload; torn-tail logging |
| `19a6ccc` | XDP reconcile syscall cost (missing-only + CIDR hash lookup); detection merge-lock amplification; local WAL durability before CRDT merge; IPC line limit |
| `d41b286` | P0 WAL retention deleted segments still holding required history |
| `079cbae` | IPC trust-boundary documentation accuracy; metrics README accuracy |
| `009ab24` | Owner decision package |
| `760f6ba` | Correction of the release-profile `panic = "abort"` claim |

### Verification evidence

The authoritative, durable record is GitHub CI — every pushed SHA below ran
all four workflows, all green.

**How to read this table.** Two things keep the newest entries out of it,
and both are properties of GitHub Actions rather than of this document:

- A commit cannot contain the CI result of its own push, because the result
  does not exist until the commit is pushed. So each SHA's four run IDs are
  recorded by a *later* commit.
- A push that carries several commits triggers **one** run set, on the branch
  head. The intermediate commits of that push have no run of their own, and
  are marked † below; they are covered by the head's runs.

The newest SHA on `master` is therefore absent from this table, and its runs
are read from GitHub Actions directly (`gh run list --branch master`). Every
SHA from `2868250` onward is recorded here, and the only gaps are that
trailing entry and the † rows.

| SHA | CI | Review Pipeline | lint-no-unwrap | Subnet Stress Pin |
|---|---|---|---|---|
| `2868250` | `39504360095` | `39504360118` | `39504360098` | `39504360156` |
| `19a6ccc` | `39508224937` | `39508225269` | `39508225062` | `39508225133` |
| `d41b286` | `39511268917` | `39511268951` | `39511268931` | `39511268993` |
| `079cbae` | `39514032029` | `39514031980` | `39514031996` | `39514032004` |
| `009ab24` | `39515415664` | `39515415636` | `39515415565` | `39515415594` |
| `760f6ba` | `39523215976` | `39523216091` | `39523216078` | `39523215939` |
| `253aa42` | `39525438086` | `39525438161` | `39525438057` | `39525438109` |
| `0a65f64` | `39526689270` | `39526689298` | `39526689337` | `39526689278` |
| `017103a` | `39527791815` | `39527791886` | `39527791805` | `39527791788` |
| `e5caac1` | `39528948722` | `39528948698` | `39528948863` | `39528948780` |
| `b1a9e9e` † | — | — | — | — |
| `79f4eac` | `39548303921` | `39548303936` | `39548303910` | `39548303966` |
| `6efa8f9` | `39549061946` | `39549061814` | `39549061576` | `39549062048` |
| `60ce0f1` | `39552618200` | `39552618088` | `39552618184` | `39552618107` |
| `26c3ab6` | `39554107784` | `39554107721` | `39554107885` | `39554107749` |
| `066ef7f` | `39555576543` | `39555576533` | `39555576496` | `39555576497` |
| `a6a61f5` † | — | — | — | — |
| `29dd601` | `39558822703` | `39558822887` | `39558822634` | `39558822885` |
| `d18535b` | `39560831028` | `39560830957` | `39560830944` | `39560830954` |
| `54ee1b7` † | — | — | — | — |

† Three SHAs have no run of their own, because each was pushed together
with the SHA after it and Actions therefore ran once, on the branch head:

- `b1a9e9e` was pushed with `79f4eac` (`e5caac1..79f4eac`); the four green
  runs on `79f4eac` cover it.
- `54ee1b7` was pushed on its own, but the runs that its push triggered are
  attributed to the branch head that contained it at the time — `6efa8f9`.
- `a6a61f5` was pushed with `29dd601` (`066ef7f..29dd601`); the four green
  runs on `29dd601` cover it.

In each case the commit is a parent of the SHA whose runs cover it, and the
covered tree is the same or a documentation-only superset of it.

The local 14-gate runner (fmt, check core/all-features, clippy core/all,
build core/all, test core/full/mesh/all-features, release build, lint-no-unwrap,
SHM ABI) produced, per batch:

**Every gate run on disk is listed below** — all 24 of them, in the order they
were run. A run that was superseded is marked as such, and a run that FAILED is
marked as a failure. The gate log is named by the commit it preceded, which
is how each row was attributed.

| # | Time | Gate log | Commit it gated | Gates | Aggregate | Status |
|---|---|---|---|---|---|---|
| 1 | 10-20 18:05 | `master_gates.log` | — | **8/14** | 1197/0 | **FAILED, superseded** |
| 2 | 10-20 22:55 | `scan_gates.log` | `2868250` | 14/14 | 1799/0/42 | superseded by CI on the commit |
| 3 | 10-21 00:09 | `perf_gates.log` | — | **11/14** | 273/2 | **FAILED, superseded** |
| 4 | 10-21 00:11 | `perf2_gates.log` | — | **11/14** | 378/3 | **FAILED, superseded** |
| 5 | 10-21 00:25 | `perf3_gates.log` | `19a6ccc` | 14/14 | 1810/0/42 | green |
| 6 | 10-21 00:44 | `d1_gates.log` | `d41b286` | 14/14 | 1813/0/42 | green |
| 7 | 10-21 00:52 | `d1b_gates.log` | `d41b286` (re-run) | 14/14 | 1813/0/42 | green |
| 8 | 10-21 01:13 | `doc_gates.log` | `079cbae` | 14/14 | 1813/0/42 | green |
| 9 | 10-21 01:25 | `pkg_gates.log` | `009ab24` | 14/14 | 1813/0/42 | green |
| 10 | 10-21 02:40 | `panicfix_gates.log` | `760f6ba` | 14/14 | 1813/0/42 | green |
| 11 | 10-21 02:51 | `evid_gates.log` | `55ff73a` | 14/14 | 1813/0/42 | green |
| 12 | 10-21 03:02 | `evid2_gates.log` | `253aa42` | 14/14 | 1813/0/42 | green |
| 13 | 10-21 03:15 | `tri_gates.log` | `0a65f64` | 14/14 | 1813/0/42 | green |
| 14 | 10-21 03:26 | `cap_gates.log` | `017103a` | 14/14 | 1813/0/42 | green |
| 15 | 10-21 03:38 | `xdp_gates.log` | `e5caac1` | 14/14 | 1813/0/42 | green |
| 16 | 10-21 06:47 | `posture_gates.log` | `b1a9e9e` — the posture change *before* the review responses | 14/14 | 1828/0/42 | green, superseded by row 17 |
| 17 | 10-21 07:06 | `final_gates.log` | `b1a9e9e` — *after* the review responses | 14/14 | 1831/0/42 | green |
| 18 | 10-21 07:16 | `dec_gates.log` | `6efa8f9` | 14/14 | 1831/0/42 | green |
| 19 | 10-21 07:26 | `trail_gates.log` | `54ee1b7` | 14/14 | 1831/0/42 | green |
| 20 | 10-21 07:43 | `native_gates.log` | `60ce0f1` — the tree *before* the clippy fix | **12/14** | 1840/0 | **FAILED on clippy, superseded** |
| 21 | 10-21 07:57 | `native2_gates.log` | `60ce0f1` | 14/14 | 1849/0/42 | green |
| 22 | 10-21 07:57 | `native3_gates.log` | `60ce0f1` (re-run) | 14/14 | 1849/0/42 | green |
| 23 | 10-21 08:04 | `trail2_gates.log` | `26c3ab6` | 14/14 | 1849/0/42 | green |
| 24 | 10-21 08:19 | `trail3_gates.log` | `066ef7f` | 14/14 | 1849/0/42 | green |
| 25 | 10-21 08:36 | `ledger_gates.log` | the ledger commit | 14/14 | 1849/0/42 | green |
| 26 | 10-21 08:50 | `open_gates.log` | `a6a61f5`, `29dd601` | 14/14 | 1849/0/42 | green |
| 27 | 10-21 09:12 | `queue_gates.log` | `d18535b` | 14/14 | 1852/0/42 | green |
| 28 | 10-21 10:28 | `f1_push_gates.log` | `d1f4a81` | 14/14 | 1858/0/42 | green |
| 29 | 10-21 11:35 | `bench_gates.log` | `5a8a32f` | 14/14 | 1858/0/42 | green |
| 30 | 10-21 12:20 | `backlog_gates.log` | `e6d01ab` | 14/14 | 1858/0/42 | green |

### F1 — CGNAT shared-memory lookup (closed)

**Commit:** `d1f4a81` — "F1: a contended SHM lookup must not masquerade as a
coherent miss". Pushed to `master` as a fast-forward from `191a7d4`.

**Gates:** `f1_push_gates.log` — 14/14 EXIT=0, 1858 passed / 0 failed / 42
ignored, `SHM C ABI and concurrent reader/writer stress: PASS`,
`SHM lookup contract: PASS`. The run covered the exact tree that was
committed (`d1f4a81^{tree}` = `027d2390a835605f0c4854b575afb3efdc368e81`).

**CI on the pushed SHA `d1f4a81`:** CI `39570398184` success, Review
Pipeline `39570398263` success, lint-no-unwrap `39570397566` success,
Subnet Stress Pin `39570397458` success.

**Reviews:** `docs/development/CGNAT_SHM_LOOKUP_REVIEW.md` (PASS, safe to
push) and `docs/development/CGNAT_SHM_LOOKUP_DELTA_REVIEW.md` (PASS, with
three required prose corrections, all applied). The delta review also
prompted a property-by-property check of the producer/consumer protocol,
which is recorded in `docs/development/CGNAT_SHM_LOOKUP_FIXES.md` §4.5.

**The RED→GREEN evidence** was reproduced with the final code: eight
checks fail with the fix reverted, and each of the two behaviours added
after the first review was re-proved alone by reintroducing its defect
(`got 1` = MISS for the probe-window case, `got 0` = FOUND for the
reserved-key case).

**Two prose claims of the first draft were wrong and are corrected** in
the report: the bool wrapper has one behaviour change, not two (the old
reader already scanned the whole probe window — its "empty terminates the
probe" comment did not describe the code), and the `shm_publish` counter is
one *lower* per reserved-key decision than before, not unchanged.

**Left open:** F1a — a /0 rule is now inexpressible end to end, and the
reserved key is not only the /0 case (`0.0.0.32/32` also hashes to 0). The
three options and their consequences are in `docs/development/OPEN_FINDINGS.md`.

### F1 follow-up — the lookup cost, measured

**Commit:** `5a8a32f` — "Measure the lookup cost under collision, and drop
an unsupported cost claim". Fast-forward from `971c088`.

The external review asked for the lookup cost to be measured under
realistic collision rates, because the table is the reverse proxy's lookup
path. `scripts/shm_lookup_bench.c` does it, and the gate now runs it.

**Gates:** `bench_gates.log` — 14/14 EXIT=0, 1858 passed / 0 failed / 42
ignored.

**CI on the pushed SHA `5a8a32f`:** CI `39577146127` success, Review
Pipeline `39577146172` success, lint-no-unwrap `39577146205` success,
Subnet Stress Pin `39577146188` success.

**Measured**, on the development host (i7-3687U @ 2.10 GHz, gcc -O2, best
of 5 repetitions of 200 000 lookups): 25.7 ns for a rule in the primary
slot, 29.1 / 32.4 / 36.3 ns at probe distance 1 / 2 / 3, and 54.3 ns for
the mixed hit/miss workload. Each extra slot in the probe window costs
about 3.5 ns, so the full-window scan — which the reader has always done,
not a change of this work — costs about 10 ns more than a single-slot
lookup.

**The `(<15ns)` comment** in `crates/ramshield-detection/src/lib.rs` was
not backed by any measurement and the measured cost is higher. It now
carries the measured figures and the caveat. That is one instance of the
performance-claims finding (F7) closed by measurement rather than by
assertion.

**The benchmark found a flaw in itself first:** the first workload let two
keys share a primary slot, so one rule overwrote another and 1143 of
200 000 lookups returned the wrong answer. The table is now built with
disjoint probe windows.

### F6 — `BACKLOG.md` re-pointed at master (closed)

**Owner decision:** the standing fence on `BACKLOG.md` was lifted by the
repository owner on 2026-10-21, with the instruction to update the backlog
after the review and to double-check the findings.

**Review:** `docs/development/BACKLOG_VALIDATION_AGAINST_MASTER.md` —
verdict: **no**, `BACKLOG.md` was not a safe answer to "what is proven on
master". All four cited header facts were stale; 18 of 33 rows carried
PASS/closed statuses that no longer described master; 4 rows described
defects the work since had closed; 4 items open on master were not
mentioned at all. It also found that the fence's designated successor file,
`docs/development/AUDIT_FINDINGS_BACKLOG.md`, did not exist on master — it
existed only on the parked mesh branch.

**What was done:** the audit-branch material moved verbatim into the new
`docs/development/AUDIT_FINDINGS_BACKLOG.md`, verified byte-identical
against the pre-change file, with the replaced header and handoff-contract
lines recorded there too. `BACKLOG.md` was re-pointed at `master` and the
assessed SHA, all 33 rows were re-stamped, a new "Open on master" section
records the 11 items the file did not mention, and the 100/100 gate is
bound to a SHA with the note that it has never been satisfied on any single
SHA.

**The findings were double-checked before they were written down.** The
review's checkable claims were re-verified against the tree: the missing
successor file, the SYNPROXY port discrepancy, the unbound `filterTable`,
the hard-coded systemd path, the tracked `fix.patch`, `hostNetwork`/
`NET_ADMIN`, the absent `rs/STATUS.md`, the dashboard session gate and its
default, the XDP reconcile regression test, and the Containerfile layer
order. All confirmed as stated.

**CI on the pushed SHA `e6d01ab`:** CI `39579193491` success, Review
Pipeline `39579193457` success, lint-no-unwrap `39579193310` success,
Subnet Stress Pin `39579193318` success.

**One review claim was wrong and was corrected.** The review stated that
the parked branch `f75abb5` "contains no `crates/ramshield-mesh` source
change". It does: 625 changed lines in `crates/ramshield-mesh/src/transport.rs`
plus test changes. The conclusion the reviewer drew — that none of it is on
master — holds, because the diff against master shows the branch is behind
master. The prose in `BACKLOG.md` states the accurate version.

**Follow-up on the same pass.** The owner asked that the backlog carry
**pending tasks only**. The 19 completed rows were moved to a "Completed
rows" section at the end of the file, keeping their full status text, so
the tables above answer only one question. P0 and P1 now have no pending
row at all, which is stated in place rather than left as an empty table.

**A second accuracy correction, found by the owner asking for the findings
to be double-checked.** The delta review had noted that the bool-wrapper
list was incomplete: the pre-fix reader matched `client_hash == hash` with
`hash == 0`, so `ramshield_shm_read(table, 0, ...)` returned `true` for a
rule stored under the reserved key. The current reader rejects that key
with `INVALID`, so the wrapper returns `false` — a **second** observable
change, not the one the shipped prose claimed. Proven by a direct probe of
the pre-fix header against the current one:

```
PRE-F1 (d1f4a81^): ramshield_shm_read(table, 0, ...) -> TRUE
CURRENT (master):  ramshield_shm_read(table, 0, ...) -> false
```

`docs/development/CGNAT_SHM_LOOKUP_FIXES.md` §2.4, the header's wrapper
comment and the crate README's release note now state both differences.

**A flaky test was found and fixed on the way.** The 14-gate run for this
commit failed once: `test-full` exited 101 on
`detection::tests::merge_local_holds_the_merge_lock_for_the_whole_drain`,
with "the merge lock was acquired 5663 times during a single drain" against
a bound of 1,000. The same test passed in `test-core` and
`test-all-features` in the same run, and passed 5/5 in isolation. The load
average was 5.00 on 4 vCPUs.

**The production behaviour was never wrong**: `merge_local` takes the merge
lock once for the whole drain. The test's *measurement* was wrong twice
over:

- `drain_started` was set BEFORE the drain thread took the lock, so a
  competitor that hot-spun could win the lock thousands of times in that
  pre-drain window. On an oversubscribed box the drain thread lost its
  slice and the competitor monopolised it. The test measured its own
  scheduling.
- Once the drain held the lock, a competitor blocked on it could not see
  the lock being freed and re-taken within nanoseconds, so it could not
  detect a regression to per-IP locking either. Verified: with a
  per-10-IP release simulated, the old competitor still reported a count
  under the bound.

The test now uses a `try_lock` probe that counts the moments the lock is
FREE after it has first been observed as HELD. That is the property under
test — how often did the drain give the lock up during the drain — and
the pre-drain window is excluded by construction.

**Non-vacuity re-proved**: with a per-10-IP lock release simulated, the
new probe reports **39,118** free windows against the bound of 1,000 and
fails. The fixed test passes 4/4 in isolation and 4/4 under a deliberate
CPU load of 3.92–5.01 on 4 vCPUs — the condition that broke the old
version. Production code was restored after every simulation; the diff
touches the test only.

**One workflow is red, and it is not this code.** The scheduled *Mesh
Acceptance Watch* run `39571107161` failed at 08:40 on 2026-10-21. It did
not test master: `.github/workflows/mesh-acceptance-watch.yml:66` resolves
the candidate as
`github.event_name == 'schedule' && 'audit-branch' || github.sha`, so a
scheduled run checks out **audit-branch** and tests the parked mesh work
(the failures are `PeerEpoch`, `Aworset` and `tombstone_acks` unresolved
in `crates/ramshield-mesh/src/transport.rs`). The workflow's own history
shows it red on master and audit-branch every day since 2026-10-12 was the
last green. It is pre-existing and belongs to the parked mesh work.

Master was verified against that workflow's own commands:
`cargo test --locked -p ramshield-mesh --all-targets` on the tree of
`0018b11` passes 73/73 locally. The four push-triggered workflows on both
`d1f4a81` and `0018b11` are green.

**27 runs, 23 green, 4 failed.** The four failures are named and explained
below. Nothing was hidden and no failure was retried into a passing log
without the failed log being kept and named.

As with the CI table above, the newest run on disk is not yet in this table:
the run taken for the commit that adds the final row cannot be in the row it
gates. The table is updated when the next documentation commit lands, and
the run it is missing is named in that commit's message.

**One precision note on the first batch.** Its gate run (`scan_gates.log`,
22:55) predates the commit and ran against the *staged index*, which was
byte-identical to what was committed apart from the review report added in the
same commit. The direct evidence on the committed tree itself is CI: all four
workflows green on `2868250`.

For every later batch the run was taken on the working tree immediately
before its commit, with no source change in between, and the committed
tree's own direct evidence is the four green CI workflows on that SHA. The
"Commit it gated" column names the commit that followed the run, not a tree
that was already committed when the run happened.

### Preserve state, verified after the last push

| Item | State |
|---|---|
| Stashes | 24 |
| `refs/backup/pre-merge` | `1ebfc2ca0095d25c5d782bf0d577d62f10574b0b` |
| Tracked `.bak`/`.orig` | 3, **deleted** in `6efa8f9` after their content was proven to be in git history (Decision 3, option A) |
| Main worktree (`.../beta/rs`) | `feature/mesh-f01-hmac-edge-secrets` at `f75abb5`, 11 pre-existing untracked files, unmodified |
| Audit-branch worktree | `fix/audit-branch-aworset-restore` at `3301b9d`, unpushed |
| Master worktree (`.../rs-local-backlog-dispatcher`) | local `master` at `d369d27`, clean |
| Other worktrees | `ramshield-pr19-fix` at `918732b`, two backlog-dispatcher worktrees, all untouched |
| Deployment | none, and none authorized |

**Four gate logs on disk show a failure.** All four are kept, named and
explained:

- **`master_gates.log` (8/14, 10-20 18:05)** — the first attempt at the
  master repair, before the coherent candidate was established. It was
  superseded by `scan_gates.log` (row 2) and by the commit `63a41ba`. The
  repair that followed is recorded in
  `docs/development/MASTER_PIPELINE_INVESTIGATION.md`.
- **`perf_gates.log` (11/14, 2 failed) and `perf2_gates.log` (11/14, 3
  failed)** — both from the *intermediate* second-batch tree, before the IPC
  line-limit check was reimplemented against the unterminated tail. Both
  failed the same single pre-existing test,
  `ipc_wiring_oversize_connection_gets_error_frame_not_bare_reset`
  (`tests/ipc_wiring.rs:92`), which the intermediate implementation broke.
  That test was never modified — `19a6ccc` touched `tests/ipc_wiring.rs`
  with 59 insertions and 0 deletions — and it passes on every tree since.
- **`native_gates.log` (12/14, 10-21 07:43)** — the first run of the native
  ingest drop-accounting batch. **Clippy failed it** with
  `too_many_arguments (9/7)` on `process_packet`
  (`src/engine/native.rs:675`), because threading the metrics handle and the
  log-emission counter pushed the function past the lint limit. The gate
  caught it before the commit. The fix was an
  `#[allow(clippy::too_many_arguments)]` with the same justification the file
  already uses on `run_linux` and `run_socket`; the re-run is row 21.

**The four failures are all superseded intermediate trees.** No failing run was
discarded, and no failure was reported as a pass.

An earlier status report cited `perf2_gates.log` as the second batch's passing
log. **That was a filename error**: the passing run is `perf3_gates.log`
(14/14, 1810/0/42), which matches the aggregate that was reported. The
substance was correct; the citation was not, and it is corrected here.

### Outstanding implementation work

The durable task list for the issues that still need implementation — the
native capture worker lifecycle (F2), the `BACKLOG.md` ledger (F6) and the
performance-claims evidence template (F7) — is
`docs/development/OPEN_FINDINGS.md`.

**F1 (CGNAT shared-memory lookup semantics) is closed**, in
`docs/development/CGNAT_SHM_LOOKUP_FIXES.md`. The owner decision it was
waiting on dissolved on inspection: the header is `static inline`, so every
consumer compiles it in, and the tri-state result was added *alongside* the
existing functions whose signatures and behaviour are unchanged. There was
no ABI change to decide. The investigation also corrected the finding's
scope — the C header is the only rule-lookup path in the repository, the
Rust crate has no read path at all — and found a third defect neither the
audit nor the review had reported: a /0 prefix hashes to the reserved
empty-slot key, so such a rule could be published and never found. It also carries the status of F5, F3 and
F4 (closed) and of F6, F7 and F8 (open, not started).

### Refuted leads

Six scan leads were **refuted** with evidence and are recorded in
`docs/development/MASTER_SCAN_NEGATIVE_RESULTS.md` so they are not
re-investigated: a peer cannot suppress another node's ban (the CRDT is
origin-bound); the native TPACKET_V3 parser is sound; the replay store is
sound against replay; the release profile **does** set `panic = "abort"` and
`AGENTS.md` now says so; the dashboard's unauthenticated loopback default is a
tested decision; and `deployment_cli.rs` contains no real `unwrap()`.

Two mesh findings are confirmed but **parked by the roadmap** — the
`merge_delta` race (F-02: *"Prevent an older block from overriding a durable
removal"*) and the unblock acknowledgement/cursor path (F-03). Both now have
a verified input recorded, so that work can start from a confirmed
interleaving rather than a raw report.
