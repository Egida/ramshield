# OPEN IMPLEMENTATION WORK — tracked findings

The durable task list for the issues that still need implementation. Every
entry states the current status, the required fix, and the closure criterion,
so a later pass can pick it up without re-deriving the intent.

**Two numbering namespaces.** The **roadmap's** `F-01..F-08`
(`/home/m/Downloads/ramshield-remediation-roadmap-v1.2.md`) and the **21
October audit's** `F1..F8`
(`/home/m/Downloads/ramshield_master_audit_2026-10-21.md`) are different
lists and must not be confused. This file tracks the **audit's** numbering.
The roadmap's own `F-02` and `F-03` remain parked and unauthorised.

Each audit finding was independently verified against the source in
`docs/development/MASTER_AUDIT_2026_10_21_REVIEW.md`. The audit itself was
remote-only — its author could not clone the repository and ran no test,
clippy, C SHM, live XDP or crash-injection run — so its severities are
engineering judgments, not test results, and the review's adjusted severities
are the ones used here.

## Status

| ID | Finding | Severity | Status | Owner decision needed |
|---|---|---|---|---|
| F5 | Unauthenticated local control plane (IPC + dashboard) | P1 on `e5caac1` | **CLOSED** — `b1a9e9e` | no |
| F3 | Native ingest drops invisible | P2 | **CLOSED** — `60ce0f1` | no |
| F4 | XDP LRU eviction residual not stated | P3 (doc) | **CLOSED** — `79f4eac` | no |
| **F1** | **CGNAT shared-memory lookup semantics** | **P1/P2** | **CLOSED** — `d1f4a81`, see `docs/development/CGNAT_SHM_LOOKUP_FIXES.md` | no — the ABI question dissolved (see below) |
| **F1a** | **A /0 rule is not representable in the SHM table** | **P2** | **OPEN** — a design decision for the owner, not a defect | no — the three options are recorded below |
| **F2** | **Native capture worker lifecycle** | **P2** | **OPEN** | **yes — restart vs. degrade vs. refuse readiness** |
| F6 | `BACKLOG.md` is not a reliable master ledger | P2 (doc) | OPEN | no — but the file is fenced by a standing rule |
| F7 | Performance/RSS claims need workload evidence | P2 (process) | OPEN | yes — the release-claims policy |
| F8 | The audit was not locally reproduced | n/a (method) | not actionable | no |

---

## F1 — CGNAT shared-memory lookup semantics

**Status:** CLOSED — implemented, gated, reviewed and pushed as `d1f4a81`.
The report is `docs/development/CGNAT_SHM_LOOKUP_FIXES.md`; the two
independent reviews are `CGNAT_SHM_LOOKUP_REVIEW.md` and
`CGNAT_SHM_LOOKUP_DELTA_REVIEW.md`.

**The ABI question dissolved.** It was flagged as needing an owner
compatibility decision, on the reasoning that a tri-state return type
changes a published C API. It does not: the header is `static inline`, so
every consumer compiles it in. The tri-state was added **alongside** the
existing functions, whose signatures and behaviour are unchanged — `true` only
on a match, `false` otherwise, exactly as before. No consumer breaks, and
there is no ABI change to decide.

**The scope was corrected by the investigation.** The C header is the only
rule-lookup path in this repository; the Rust `ramshield-cgnat` crate has
**no read path at all** — it only publishes, and `get_slot` is used inside
`publish_rule`, in one test and one bench. So the earlier review's suggested
"align the Rust read path" step is a no-op, and the review's hedge about
that path being unverified resolves into something stronger.

**A third defect was found that neither the audit nor the review had
reported:** a /0 prefix masks to network 0, which hashes to key 0 — the
reserved empty-slot sentinel — and the publisher treats a zero-hash slot as
free. A rule published under that key sat in a slot every other publisher
considered empty, so the next colliding publish overwrote it, and a
prefix-0 lookup was routed to key 0 with no error. The publisher now
refuses the reserved key, and the lookup rejects it as `INVALID` rather
than routing it into the table.

The reserved key is not only the /0 case: the slot-key mixer is
0-preserving, so a key is 0 exactly when the masked network address equals
its prefix length — every /0, and `0.0.0.32/32` among others.

**The original finding, for the record:**
**Files:** `crates/ramshield-cgnat/include/ramshield_shm.h`, issue `#30`,
draft PR `#32`
**Severity:** P1/P2

### The defect

The C reader still returns a boolean. It retries a bounded number of times
when the sequence counter is odd or changes during a read, but it ultimately
returns `false` when it cannot obtain a coherent snapshot.

That means the API still cannot distinguish a genuine miss from a contended
read.

The IPv4 wrapper also still converts an invalid prefix length greater than 32
to key zero and passes that key into the generic lookup.

The independent review adds an important qualification: **no in-repository
production caller of this C header was established.** The published C API
still has the contract defect, but the separate Rust SHM implementation must
not be assumed to share it without inspecting that implementation
independently.

### Required fix

1. Give callers distinct `FOUND`, `MISS`, `CONTENDED` and invalid-input
   results.
2. Reject invalid prefixes before lookup.
3. Audit all callers and define their response to contention.
4. Preserve bounded retries.
5. Add deterministic contention and prefix-boundary tests.
6. Verify C/Rust ABI layout and run the relevant SHM stress gates.

### Closure criterion

A contended lookup cannot masquerade as a coherent miss, and the result SHA
has passing C/Rust regression tests plus independent review.

### Notes for whoever picks it up

- **Do not count draft PR `#32` as resolved** merely because its title
  describes the intended fix. It is a draft, and its own description says it
  has not been locally compiled or tested in that orchestration pass.
- The shape of the tri-state result is a **published C ABI change**. It needs
  an owner compatibility decision before the ABI shape is committed — this is
  the one place where the review flagged a decision that is genuinely the
  owner's to make, not a mechanical choice.
- The Rust implementation in `crates/ramshield-cgnat/src/shm.rs` is a separate
  implementation and was **not** verified by the review. Inspect it
  independently; do not assume the defect is present or absent there.
- The `shm-abi` gate in the local 14-gate runner and the C ABI/stress checks
  in CI are the relevant gates.

---

## F1a — a /0 rule is not representable in the SHM table (residual of F1)

**Status:** OPEN — a design gap, deliberately not fixed in F1.

F1 closed the lookup contract, and it closed the reserved-key hazard by
refusing that key on both sides: the publisher refuses `client_hash == 0`
and the reader rejects it with `INVALID`. The consequence is that **a /0 rule is now inexpressible end
to end**.

That is correct for a table whose key space reserves 0, but it means a
deployment that needs a *global* block has no path into this table. The
in-repo enforcement path is unaffected — a /0 CIDR block is still enforced
through the normal channel, and the userspace path remains authoritative.

**Why it was not fixed here:** supporting /0 needs a different key
derivation, not a patch. The reserved sentinel and the key space are
coupled in the on-disk layout, so changing the derivation is a format
decision, and the format is shared with an out-of-repo consumer.

**The three options, with their consequences.** This is a format decision
for the owner, not a patch:

| option | what it means | consequence |
|---|---|---|
| **Retain the restriction** | `/0` stays unsupported in this side-channel format | nothing changes in the format. The consumers must be told, and the in-repo enforcement path keeps covering `/0`. Cheapest and safest. |
| **Change the representation** | give `/0` an explicit encoding that does not collide with the reserved key | the key derivation changes on both sides and the parity vectors are re-pinned. The format is shared with an out-of-repo consumer, so this needs **coordinated compatibility work** — the consumer must be updated in the same release, or it will look up the wrong slot. |
| **Use a separate representation** | carry a default-route rule in its own field or mechanism rather than in the table | the table format stays frozen, but a second mechanism has to be defined, published, read and enforced by the consumer. More moving parts, no compatibility break. |

What must not happen is silently reinterpreting the reserved key: a
consumer outside this repository would keep using the old key space.

**Required fix, when it is wanted:** the owner picks an option, then the C
and Rust derivations change together and the parity vectors are re-pinned.
This is a follow-up, not a defect in the shipped behaviour.

---

## F2 — Native capture worker lifecycle

**Status:** OPEN
**Files:** `src/engine/native.rs`, `src/engine/boot.rs`
**Severity:** P2

### The defect

The drop-accounting fix (`60ce0f1`) is good, but it addresses a different
problem from worker lifecycle.

Current master still:

- collects thread-spawn results and uses `handles.into_iter().flatten()`,
  silently discarding thread-creation errors;
- logs a warning when a worker's capture function exits with an error, then
  allows that worker to terminate;
- discards thread join results;
- lacks a durable capture-health signal and a defined restart or degraded-state
  policy.

The supervisor itself can start successfully even though its workers
subsequently fail to start.

**Why it matters:** the detection-worker startup path already refuses
readiness when those workers fail (`src/engine/boot.rs` — detection refuses
ready state), while the native capture path does not provide equivalent
assurance. That asymmetry is the defect.

### Required fix

1. Handle every worker-spawn result explicitly.
2. Track expected, started, running and failed workers.
3. Define bounded restart behavior or a clear failure policy.
4. Expose native capture status through existing health/metrics mechanisms.
5. Decide whether total capture failure should refuse readiness or mark the
   service degraded.
6. Test partial startup failure, total failure, runtime exit, shutdown and
   restart exhaustion.

### Closure criterion

Capture cannot silently disappear while the service continues to imply that
the configured subsystem is functioning.

### Notes for whoever picks it up

- This is an **observability and lifecycle defect** in the optional
  native-ingest path, **not** evidence of a memory-safety vulnerability.
  `native_ingest.enabled` defaults to `false`, so the blast radius is limited
  to deployments that enable it.
- It should be prioritized **after** the CGNAT fix (F1).
- Item 5 is a **policy decision for the owner**: restart the worker, fail the
  feature, or degrade the service. The in-repo precedent is the detection
  path, which refuses readiness when its workers fail to start — following
  that precedent is a defensible default, but it changes the daemon's startup
  contract and so is worth an explicit decision rather than an inference.
- The metrics surface added by `60ce0f1` (`ramshield_native_ingest_drops_*`)
  is the natural place to hang a capture-health gauge.

---

## F6 — `BACKLOG.md` is not a reliable master release ledger

**Status:** OPEN (P2, documentation)
**File:** `BACKLOG.md`

The checked-in header still describes `audit-branch` as the target branch and
cites three superseded SHAs. PASS rows are pinned to a SHA that is no longer
master, so the file is useful as historical task context but unsafe as the
sole answer to "what is proven on current master?".

**Required fix:** re-point the header at `master` and the current exact SHA;
split the historical `audit-branch` notes into a clearly labelled appendix;
re-stamp every current PASS row with its exact SHA and workflow URLs.

**Not started:** a standing rule fences this file — it must not be
overwritten, and `docs/development/AUDIT_FINDINGS_BACKLOG.md` is the separate
home for new work. This entry needs that fence lifted before it can be
actioned.

---

## F7 — Performance and memory claims require workload evidence

**Status:** OPEN (P2, process — no code change)
**Files:** `config.baseline.toml`, issue `#35`, draft PR `#34`

The baseline config's `ram_limit_mb = 1024` and `pre_aggs_max_size = 1_000_000`
are **configuration ceilings**, not measured RSS. The "~100 MB working set"
wording is a sizing rationale, not an observed figure.

**Required fix:** before any RSS or throughput figure is published, record the
exact SHA, build profile and features, toolchain and config hash; the machine,
CPU, RAM, kernel, driver and NIC; the workload generator, packet sizes and
rates, unique-source cardinality and duration; process RSS and peak RSS
separately from kernel/XDP maps and page cache; CPU, latency, drops, detection
quality and recovery behaviour; and repeated runs with raw output. Keep
sub-20ns microbenchmark language out of public claims unless a reproducible
benchmark supports it on the actual implementation and target machine.

**Not started:** the audit's claim that PR `#34` is a draft describing 50 MB
as a target was **UNVERIFIED** by the review (a remote-only artifact). The
reviewer did not contradict it but did not confirm it.

---

## F8 — The audit was not locally reproduced

**Status:** not actionable against the code.

The audit is accurate about its own method: it states that it did not run
`cargo test`, `cargo clippy`, the C SHM tests, the live XDP tests or
crash-injection tests. The practical consequence is that its P1/P2 labels are
unverified judgments, and it caps the confidence of every other finding.

**Before any P1 item is closed**, require the corresponding gate on one exact
SHA and treat the audit's severity labels as hypotheses.
