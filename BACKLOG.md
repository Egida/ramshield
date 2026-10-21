# RamShield Production-Essential Audit Backlog

**Target branch:** `master`

**Assessed at:** `5a8a32f2a5fddf9ac7c0054c3a212b060899ce9c` (== `origin/master`)

**Latest recorded CI on that SHA:** CI [39577146127](https://github.com/grep999/ramshield/actions/runs/39577146127), Review Pipeline [39577146172](https://github.com/grep999/ramshield/actions/runs/39577146172), lint-no-unwrap [39577146205](https://github.com/grep999/ramshield/actions/runs/39577146205), Subnet Stress Pin [39577146188](https://github.com/grep999/ramshield/actions/runs/39577146188) — all success. Local 14-gate run on the same tree: 14/14 EXIT=0, 1858 passed / 0 failed / 42 ignored (`bench_gates.log`). **A commit cannot carry the CI result of its own push**, so read the per-SHA record for every later SHA in the ledger below.

**Per-SHA evidence ledger:** [`docs/development/OWNER_DECISION_PACKAGE.md`](docs/development/OWNER_DECISION_PACKAGE.md) — every gate run, every CI run ID, every SHA, including the four runs that failed and were superseded. The ledger is the answer to "what is proven on which SHA"; this file is the answer to "what is the status of each row".

**Durable open-work queue:** [`docs/development/OPEN_FINDINGS.md`](docs/development/OPEN_FINDINGS.md) — the live queue, with a closure criterion and the owner decision each open item needs.

**Historical record:** [`docs/development/AUDIT_FINDINGS_BACKLOG.md`](docs/development/AUDIT_FINDINGS_BACKLOG.md) — the audit-branch remediation passes and the 2026-10-17 audit import, moved verbatim. It is history, not a ledger.

**Operating rule:** one bounded fix per pass; every fix is revalidated on the exact resulting SHA. No merge, force-push, rebase, or changes to `kiddo_fix` / PR #3 / PR #5.

**Scope:** this backlog tracks only production-essential fixes for the core RamShield product. Mesh feature design and in-development mesh implementation work live in [`docs/development/MESH_FEATURE_PLAN.md`](docs/development/MESH_FEATURE_PLAN.md) and are not core release blockers while mesh remains optional and disabled. Any mesh code enabled in a deployment must still meet its own acceptance gate before production use.

**Meaning of 100/100:** every applicable production-essential P0/P1 is closed with regression evidence, P2s are closed or explicitly justified as non-applicable, all core acceptance gates pass on one SHA, and an independent reviewer approves that SHA. A green build alone is not 100/100.

**State review (2026-10-21, re-pointed at master):** this file previously named `audit-branch` as its target, three superseded SHAs as its baselines, and a CI run from before every finding closed since. That is fixed. The rows below are re-stamped against `5a8a32f`. Four rows are now closed by named commits (I-001, I-002, I-004, I-010), three are re-stamped on the current SHA with the evidence that supports them (A-030, A-031, A-033), and the rest carry an honest label — `PASS (historical)` for a pass that was never re-verified on current master, or `OPEN` for a row verified still open. The independent review that produced this re-stamping is [`docs/development/BACKLOG_VALIDATION_AGAINST_MASTER.md`](docs/development/BACKLOG_VALIDATION_AGAINST_MASTER.md).

## Priority definitions



- **P0 — release blocker:** security boundary, enforcement correctness, data corruption/loss, or unrecoverable failure.

- **P1 — required before production claim:** bounded resource use, lifecycle/recovery, correctness under concurrency/partial failure, or missing essential test coverage.

- **P2 — quality completion:** reproducibility, maintainability, docs, and coverage needed to make the score defensible.

- **Status vocabulary:** `OPEN`, `IN PROGRESS`, `PASS (exact SHA)`, `PASS (historical)`, `CLOSED (commit)`, `BLOCKED`, `NOT APPLICABLE (evidence)`, `UNVERIFIABLE`. Do not mark PASS from source inspection alone.

  - `PASS (exact SHA)` — the row's substance was verified on that exact SHA, with the evidence named.
  - `PASS (historical)` — the row passed at a SHA that is now an ancestor of master, and its substance has **not** been re-verified on current master. It is a record of a past result, not a current claim.
  - `CLOSED (commit)` — closed by the named commit, which carries its own evidence and review.
  - `UNVERIFIABLE` — the row names a path or artifact that does not exist in the current tree, so it can be neither confirmed nor dismissed as written.



## P0 — security and correctness blockers



| ID | Area | Required work / acceptance evidence | Status |

|---|---|---|---|

| A-004 | Shared-memory writer parity | Root `publish_rule` uses exclusive even→odd CAS + under-lock revalidation. C `ramshield_shm_clear_slot`/`flush_all` mirrors CAS protocol. Concurrent writer and C reader/writer stress tests added. C/Rust layout checks cover size, alignment, and field offsets; ThreadSanitizer passed where supported. Nested `rs/` out of scope (A-021). Exact-SHA CI and Backlog Gate passed on `a9a3bf16c47680108993b75bf81b4b7e59bb31f8`. | PASS (historical) — not re-stamped. The SHM contract changed after this pin: the published reader is now tri-state and the reserved key is refused on both sides (`d1f4a81`; see I-001 and I-002). |

| A-005 | Replay-store security | Live markers are never LRU-evicted under global/per-key pressure; saturation returns `capacity`. Time sampled under lock; poison fails closed. Tests: unit module + `crates/ramshield-protocol/tests/replay.rs`. Ported security model aligned with kiddo_fix review. Exact-SHA CI and Backlog Gate passed on `a9a3bf16c47680108993b75bf81b4b7e59bb31f8`. | PASS (historical) — not re-verified on current master. No commit since `a9a3bf1` touched the replay store, and the master scan lists the replay store as a sound, refuted lead. |



## P1 — boundedness, failure handling, and operational correctness



| ID | Area | Required work / acceptance evidence | Status |

|---|---|---|---|

| A-015 | Detection construction | Public `new` remains a deprecated compatibility wrapper; production boot uses `try_new`, and `try_new_with_shm_path` supports deterministic initialization tests. `try_new_propagates_shm_open_failure` verifies the invalid SHM path returns `Err` without panic. Exact-SHA CI and Backlog Gate passed on `a9a3bf16c47680108993b75bf81b4b7e59bb31f8`. PR #6's `#[cfg(test)]` removal of the public wrapper is a downstream API break and requires an explicit breaking-release decision. | PASS (historical) — not re-verified on current master. The row's own caveat is unresolved: PR #6's `#[cfg(test)]` removal of the public wrapper is a downstream API break and needs an explicit breaking-release decision. |

| A-016 | Native packet ingest | TPACKET_V3 validators and malformed-descriptor property sweeps cover block/header/packet bounds, ring edges, chain alignment/overflow, and deterministic malformed mixes. Exact-SHA CI and Backlog Gate A-016 TPACKET bounds passed on `a9a3bf16c47680108993b75bf81b4b7e59bb31f8`. | PASS (historical) — not re-stamped. Changed since this pin: the native capture path now counts drops and documents `max_events_per_sec` as an admission cap rather than a delivery guarantee (`60ce0f1`, `d18535b`). |

| A-017 | XDP/SYNPROXY operational correctness | Tests cover capacity rejection, reconcile-after-map-loss (v4/v6/CIDR), partial-install failure isolation, StubXdp no kernel effects, IPv4/IPv6 and CIDR map routing, native-mode no-downgrade, and SYNPROXY unsupported-environment / hostile-interface / cleanup behavior. Exact-SHA CI and Backlog Gate passed on `a9a3bf16c47680108993b75bf81b4b7e59bb31f8`. | PASS (historical) — not re-stamped. Changed since this pin: the reconcile path was rewritten (`2868250` hang fix, `19a6ccc` syscall cost). The row also does not mention the two bounded LRU residuals — CIDR fail-closed and per-IP fail-open, up to 10 s each (`79f4eac`; `docs/development/MASTER_SCAN_NEGATIVE_RESULTS.md` 7g). |

| A-018 | WAL / checkpoint recovery | Crash/restart test truncates to the last explicit sync boundary and asserts the exact durable prefix; recovery tests prove torn-tail repair is idempotent and the repaired WAL accepts later appends. Added disk-full-after-checkpoint injection proving the checkpoint and durable prefix survive, failed append is excluded, and reopen accepts new writes. Exact-SHA CI and Backlog Gate passed on `a9a3bf16c47680108993b75bf81b4b7e59bb31f8`. | PASS (historical) — not re-stamped. Changed since this pin: WAL retention was fixed because it deleted segments still holding required history (`d41b286`). |

| A-019 | Dashboard / IPC auth | Session TTL expiry, cookie HttpOnly/SameSite/Secure, per-IP lockout, invalid PHC fail-closed, oversized password reject, deployment defaults (loopback, 8h TTL, lockout 50). Poisoned password-hash lock denies login and keeps auth enabled; explicit hash replacement clears poison after writing the replacement. Failed-login tracking is hard-capped at 10,000 IPs with serialized admission and a concurrent unique-source flood regression test. IPC: replay reject, malformed frames, parse_ipc_keys fail-closed. Exact-SHA CI and Backlog Gate passed on `a9a3bf16c47680108993b75bf81b4b7e59bb31f8`. | PASS (historical) — not re-stamped. The authentication model was replaced by an owner decision: a mutating IPC request without a verified principal is now 401, and `POST /api/config` requires a valid session (`b1a9e9e`; `docs/development/POSTURE_CHANGE_FIXES.md`). |

| A-020 | Resource bounds elsewhere | PulseTracker observations capped at 4,096. `pending_mitigations` enforces the 1,048,576 production / 64 test cap at admission with bounded eviction; batch-flood/concurrency tests added. `detection.pre_aggs_max_size` capped at 1,000,000; enabled forecasting seasonal allocation capped at 86,400 one-second slots; boundary and `usize::MAX` validation tests added. Existing: store capacity CAS, threat_sample≤1024, IPC conn semaphore, SHM/replay caps. Worker-local maps stop at 8,192 distinct IPs; production shared-map merges serialize with flushes and flush before admitting a new key at configured capacity. Regression test verifies the shared cap and exact event accounting. Exact-SHA CI and Backlog Gate passed on `a9a3bf16c47680108993b75bf81b4b7e59bb31f8`. | PASS (historical) — not re-stamped. The listed caps stand, but the enumeration is no longer exhaustive: native-ingest drop counters and queue-pressure metrics were added afterwards (`60ce0f1`, `d18535b`). |

| A-021 | Root vs nested workspace | **DECIDED 2026-10-10:** `rs/` is archival/experimental only — not maintained, not shipped, not root-CI-gated. Production product is the root workspace. Evidence: `docs/WORKSPACE_SCOPE.md`, `rs/STATUS.md`. Reversal requires pinned CI + parity policy (see doc). Root workspace CI and Backlog Gate passed on `a9a3bf16c47680108993b75bf81b4b7e59bb31f8`. | PASS (historical) — not re-stamped. The decision stands, but its evidence pointer `rs/STATUS.md` is gone (the nested `rs/` archive was removed). `docs/WORKSPACE_SCOPE.md` is still on master. |



## P2 — full quality and reproducibility



| ID | Area | Required work / acceptance evidence | Status |

|---|---|---|---|

| A-030 | Formatting and gates | `cargo fmt --all -- --check` is enforced in CI and passed on exact SHA `a9a3bf16c47680108993b75bf81b4b7e59bb31f8`. | PASS (exact SHA `5a8a32f2a5fddf9ac7c0054c3a212b060899ce9c`) — `cargo fmt --all -- --check` clean in the local 14-gate run on that tree (`bench_gates.log`); CI `39577146127` success. |

| A-031 | Core feature matrix | CI and Backlog Gate separately run default/core-only and production `full` / all-feature configurations: locked all-targets check/build, workspace tests, and strict Clippy. Mesh-only and mesh-integrated configurations remain in the separate mesh feature plan. Exact-SHA CI and Backlog Gate passed on `a9a3bf16c47680108993b75bf81b4b7e59bb31f8`. | PASS (exact SHA `5a8a32f2a5fddf9ac7c0054c3a212b060899ce9c`) — the local 14-gate run covered check/build/clippy/test for core, full, mesh and all-features, plus a release build (`bench_gates.log`); CI `39577146127` success. |

| A-033 | Shared-memory ABI | C header statically asserts 64-byte size/alignment and every `RamshieldShmRuleEntry` field offset; Rust layout is asserted at compile time and runtime. CI runs C pthread reader/writer stress, ThreadSanitizer, and AArch64 Rust/C cross-compilation checks. All passed on exact SHA `a9a3bf16c47680108993b75bf81b4b7e59bb31f8`. | PASS (exact SHA `5a8a32f2a5fddf9ac7c0054c3a212b060899ce9c`) — the `shm-abi` gate passed on that tree and now also runs the deterministic lookup contract and the collision-cost benchmark (`bench_gates.log`); CI `39577146127` success. The ABI surface grew a tri-state result and a reserved-key rule after the original pin (`d1f4a81`). |

| A-034 | Dependency / supply chain | CI runs `cargo audit --no-yanked` and `cargo deny check`; both passed on exact SHA `862ea51f5251d974de6d9f21f375c796282bddf0`. Workspace crates now declare MIT licenses; local path dependencies carry explicit package-version constraints; cargo-deny uses current v2 advisory policy and denies advisories, yanked crates, wildcard dependencies, and unknown sources. Reviewed production unsafe boundaries in TPACKET ring parsing, SHM mmap access, and XDP/BPF syscall helpers; `raw_get_next_key` now documents the requirement that kernel-written key bytes form a valid `K`. Remaining cargo-deny duplicate-version warnings are intentionally visible: `thiserror` v1 is used by `ramshield-types` while root/enforcement use v2; `getrandom`, `r-efi`, `rand_core`, and `syn` duplicates are transitive graph splits and are not force-unified without compatibility review. No advisory, license, or source-policy errors remain. | PASS (historical) — not re-stamped. The policy and the visible duplicate set are as described; the pin is 30+ commits behind, and the tree has since gained the SHM contract test and the collision benchmark. |

| A-035 | Public API / error semantics | `doctor` verifies `ip link show` exit status; CLI IPC enforces newline framing, a 1 MiB response cap, 5-second read/write timeouts, UTF-8, and valid JSON; CLI HMAC auth accepts a configurable key ID and enforces the server's 16-byte minimum key size; SYNPROXY ports reject malformed and non-UTF-8 environment values while an intentional blank list clears defaults. Regression tests cover these cases, CLI framing and parse errors, and doctor config/exit semantics. Dashboard Argon2 concurrency docs were corrected on `748feb2010e78a614b684b4d9cc1c4568a04fcb2`. Exact candidate `0e48c3bc6ce7997d5e57421930715706b9627f71`: full CI [run 38297139446](https://github.com/grep999/ramshield/actions/runs/38297139446) and Backlog Gate [run 38297139440](https://github.com/grep999/ramshield/actions/runs/38297139440) passed. Local core/full tests, check/build, strict Clippy, formatting, `cargo audit --no-yanked`, and `cargo deny check` also passed. The final independent audit is tracked under A-038. | PASS (historical) — not re-stamped. No commit since changed those surfaces. The release-profile claim was separately corrected on master (`760f6ba` — `panic = "abort"` IS set), which this row does not reflect. |

| A-036 | Core docs and claims | Replace stale code-health notes; document supported core deployment commands and operational assumptions. Add/update README files for analytics and CGNAT; verify enforcement and detection documentation against code. Mesh threat model, deployment, and module docs are tracked in the separate mesh feature plan. | IN PROGRESS — the documentation it describes was refreshed, but `DOCS COMPLETE` is not a status in this file's vocabulary, and the row's own acceptance ask (exact-SHA gates and review) is still open. The master record set is now far larger than this row describes. |

| A-037 | Core benchmarks and regression budgets | Establish reproducible baselines for detection, enforcement, WAL, native ingest, and memory under load. Set only evidence-based regression thresholds. Mesh merge/gossip performance is tracked in the separate mesh feature plan. | IN PROGRESS — still partly unmeasured: kernel capture/TPACKET ring and isolated memory behaviour. One cost claim is now measured rather than asserted: the SHM lookup cost, by probe distance and over a mixed hit/miss workload (`5a8a32f`; `scripts/shm_lookup_bench.c`), and the unsupported `(<15ns)` comment in `crates/ramshield-detection/src/lib.rs` was replaced with the measured figures. |

| A-038 | Final independent audit | Two independent source-only reviewers assess the same exact candidate SHA and current diff against master; each records findings, severity, evidence, and APPROVE/BLOCK. Fix all P0/P1; resolve or justify P2s. | OPEN — the gate this row describes (two independent reviewers on one exact SHA) has not been met on any current SHA. Per-change independent reviews exist — `CGNAT_SHM_LOOKUP_REVIEW.md` and `CGNAT_SHM_LOOKUP_DELTA_REVIEW.md` (F1), `POSTURE_CHANGE_REVIEW.md` (F5), `NATIVE_INGEST_DROPS_REVIEW.md` (F3), `MASTER_AUDIT_2026_10_21_REVIEW.md` (the 21 Oct scan) — but that is per-change review, not a single-SHA release approval of the whole tree. |

| A-039 | Final acceptance report | Attach exact SHA, compare-to-master, workflow URLs/job conclusions, fmt result, test counts, known limitations, module scores with evidence, and a prioritized residual-risk statement. No blanket 100/100 without this evidence. | IN PROGRESS — the acceptance evidence now exists in a newer, per-SHA form: the gate ledger and CI run table in `docs/development/OWNER_DECISION_PACKAGE.md`. It is not attached here as a single final report. |



## Separate in-development feature



Mesh is intentionally excluded from this production-essential remediation backlog. Track its feature requirements, open correctness work, development sequence, and mesh-specific acceptance criteria in [`docs/development/MESH_FEATURE_PLAN.md`](docs/development/MESH_FEATURE_PLAN.md). Do not use incomplete mesh feature work to inflate or block the core remediation score while mesh remains optional and disabled; do not enable mesh in production before its own acceptance gate passes.

**"Out of scope" does not mean "fine", and the scheduled mesh gate does not test master.** `.github/workflows/mesh-acceptance-watch.yml:66` resolves the candidate as `audit-branch` on a schedule event, so every scheduled run of the Mesh Acceptance Watch checks out the parked mesh branch and never exercises master. That workflow has been red every day since 2026-10-12. A reader of this file must not read "mesh is out of scope" as "mesh is verified". Recorded on master by `971c088`.



## 100/100 acceptance gate



A candidate is eligible only when all are true:



- Every P0 and P1 is `PASS` with a regression test or evidence-based `NOT APPLICABLE` decision.

- All P2 tasks are closed or have a reviewer-approved rationale.

- Core-only and production full-feature checks/tests, all-targets check/build, and applicable ABI/recovery gates pass on the same SHA. Mesh-specific gates are owned by the separate mesh feature plan.

- `cargo fmt --all -- --check`, strict Clippy, and `git diff --check origin/master...HEAD` pass on that SHA.

- Core security/recovery tests include malformed input, overload, concurrency, crash/restart, and failure injection where relevant.

- Two independent reviewers approve the exact same SHA; no unresolved P0/P1 finding remains.

- The final report links all run logs and states remaining environmental/coverage limits.

**This gate has not been satisfied on any single SHA to date.** The closest per-SHA evidence is the ledger in [`docs/development/OWNER_DECISION_PACKAGE.md`](docs/development/OWNER_DECISION_PACKAGE.md): every gate run, every CI run ID, every SHA, and the four runs that failed and were superseded. The gap is the "two independent reviewers approve the exact same SHA" clause — the repository holds per-change independent reviews, not a single-SHA release approval of the whole tree. The sentence above that stands: a green build alone is not 100/100.



## Workflow / handoff contract



Every pass records the base SHA, exact result SHA, one bounded selected change, changed paths, regression coverage, commands actually run with exit status, CI run/job links, remaining findings, and next action. Never force-push, rebase, or alter `kiddo_fix`, PR #3, or PR #5 as part of this workflow.

The workflow itself is recorded in [`docs/development/AGENT_FLOW_RUNBOOK.md`](docs/development/AGENT_FLOW_RUNBOOK.md), and the per-pass evidence in [`docs/development/OWNER_DECISION_PACKAGE.md`](docs/development/OWNER_DECISION_PACKAGE.md). The `audit-branch` workflow this contract originally described is history; it is in [`docs/development/AUDIT_FINDINGS_BACKLOG.md`](docs/development/AUDIT_FINDINGS_BACKLOG.md).


## Open on master as of `5a8a32f`

This file's rows are the backlog's own items. These are the things that are
open on master today and that the rows above do not describe. The durable
queue with a closure criterion for each is
[`docs/development/OPEN_FINDINGS.md`](docs/development/OPEN_FINDINGS.md).

| # | item | state |
|---|---|---|
| 1 | **F1a — a /0 rule is not representable in the SHM table.** F1 closed the lookup contract by refusing the reserved key 0 on both sides, so a /0 CIDR rule is now inexpressible end to end in this side channel. The reserved key is reached whenever the masked network address equals its prefix length — every /0, and `0.0.0.32/32` among others. Three options (retain / change the representation / a separate mechanism) need an owner decision, and the format is shared with an out-of-repo consumer. | OPEN — owner decision |
| 2 | **F2 — native capture worker lifecycle.** Spawn errors are flattened away, runtime exits are invisible, join results are discarded, there is no health signal and no restart or degraded-state policy. | OPEN — one owner answer needed: on total capture failure, refuse readiness or mark the service degraded? The detection path's refuse-readiness behaviour is the in-repo precedent. |
| 3 | **F6 — this file.** It was not a reliable master ledger: the header named `audit-branch` and three superseded SHAs, and the PASS rows were pinned to ancestors of master. | CLOSED by this re-stamp |
| 4 | **F7 — performance and memory claims need workload evidence.** The `ram_limit_mb` and `pre_aggs_max_size` values are configuration ceilings, not measured RSS, and the "~100 MB working set" wording is a sizing rationale, not an observation. | OPEN — process item. One instance is closed by measurement (`5a8a32f`, the SHM lookup cost). |
| 5 | **F8 — the 21 Oct audit was not locally reproduced.** That caps the confidence of every severity label the audit carries; this file still presents imported audit findings at face value. | OPEN — not actionable against the code |
| 6 | **The master's own non-audit fixes are not in these tables.** `2868250` (XDP reconcile hang, live-WAL read repair, `decode_hex` panic, counter overflow, SHM path hardening, IPC reason bound, panic-hook payload, torn-tail logging), `19a6ccc` (reconcile syscall cost, detection merge-lock amplification, local WAL durability before CRDT merge, IPC line limit), `d41b286` (WAL retention deleting segments still holding required history), `760f6ba` (release profile really does set `panic = "abort"`), `079cbae` (IPC trust-boundary doc accuracy). | CLOSED on master; absent from these tables |
| 7 | **The two bounded XDP LRU residuals.** A-017 is stamped historical and says nothing about them: an expired CIDR prefix lingers up to 10 s (fail-closed, the safe direction), and an LRU-evicted per-IP block is unenforced for up to 10 s (fail-open). | Documented — `docs/development/MASTER_SCAN_NEGATIVE_RESULTS.md` 7g |
| 8 | **The red Mesh Acceptance Watch.** A workflow is red every day and has not tested master since 2026-10-12, because the candidate resolves to `audit-branch` on schedule events. | OPEN — workflow fix, not a code fix |
| 9 | **The parked, unpublished F-01 branch and its disclosed deviation.** The pairwise-HMAC migration *was* implemented, on a branch, never pushed and never merged — a deviation from the roadmap's "must not be implemented unilaterally". Decision 2 (defer) is in force; the disclosure is the point. | PARKED by owner decision |
| 10 | **The four failed local gate runs.** 28 gate logs on disk, 4 of them failed and superseded — `master_gates` 8/14, `perf_gates` and `perf2_gates` 11/14 each, `native_gates` 12/14 on Clippy. "All CI jobs passed" is not the whole evidence story. | Recorded in the ledger, all four named and explained |
| 11 | **The two confirmed-but-parked mesh findings** (F-02 merge-delta race, F-03 unblock ack/cursor). Both have a verified interleaving recorded. I-003 covers the general mesh gate but not these two named defects. | PARKED by the roadmap |


## Audit import — 2026-10-17 (source: attached Comprehensive Codebase Audit & Architectural Scan)

**Handling rule:** the attached audit is a source of candidate findings, not proof that each finding still exists. Revalidate every item against the selected branch before code changes. Its module scores are provisional and must not be reported as current ratings without reproducing the review.

The dated remediation passes, the audit execution protocol, and the 2026-10-17 orchestration checkpoint that accompanied this import are in [`docs/development/AUDIT_FINDINGS_BACKLOG.md`](docs/development/AUDIT_FINDINGS_BACKLOG.md), moved verbatim. The tables below are the part that had to change: they are statements about the present.

### P0 — safety/security and fail-open correctness

| ID | Area | Required work / acceptance evidence | Status |
|---|---|---|---|
| I-001 | CGNAT SHM read under writer contention | Re-audit `ramshield_shm_read` in `crates/ramshield-cgnat/include/ramshield_shm.h`. Ensure an incoherent seqlock read cannot be treated as an empty/nonmatching slot or cause a false negative. Avoid retry logic that can underflow an unsigned probe index or spin without a bound. Add deterministic/concurrent C regression coverage, run ABI/stress gates, record exact SHA. | CLOSED (`d1f4a81`) — the published reader is now tri-state `FOUND`/`MISS`/`CONTENDED`/`INVALID`; a contended read is reported as `CONTENDED` instead of collapsing into a miss, so the fail-open this row describes cannot happen. The 64-attempt / 4-probe budget is unchanged, the reserved key is rejected before any slot is read, and the result values are pinned by static assertion. Deterministic C coverage in `scripts/shm_lookup_contract.c`, wired into the `shm-abi` gate. Report: `docs/development/CGNAT_SHM_LOOKUP_FIXES.md`; reviews `CGNAT_SHM_LOOKUP_REVIEW.md` (PASS) and `CGNAT_SHM_LOOKUP_DELTA_REVIEW.md` (PASS). |
| I-002 | CGNAT IPv4 prefix validation | Verify `ramshield_shm_read_ipv4` rejects `prefix_len > 32` before deriving/using a key. Invalid input must return a policy/input failure and must not query hash slot 0. Add tests for 0, 32, 33 and maximum integer values where the API type allows. | CLOSED (`d1f4a81`) — `ramshield_shm_read_ipv4_ex` rejects `prefix_len == 0` and `prefix_len > 32` before the key is derived or any slot is read, and returns `RAMSHIELD_SHM_INVALID`; the reserved key is rejected on the read side too. The row's test ask (0, 32, 33, maximum) is covered by the contract vectors. |
| I-003 | Mesh production safety and provenance | Keep mesh optional and disabled in production. Do not ship/enable until authenticated per-origin signatures/identity, receiver-confirmed recovery completion (generation + WAL boundary + deterministic state digest), restart/retention behavior, and multi-process fault-injection convergence gates pass on the same exact SHA with independent review. | BLOCKED — still correct, and the gate is still unmet. The pairwise-HMAC branch `feature/mesh-f01-hmac-edge-secrets` (`f75abb5`) is unpublished and not merged: no mesh transport or aworset change from it is on master. Mesh stays optional and disabled in production. No cryptographic change is proposed here — the v1.2 constraint (HMAC stays, no asymmetric signatures, no public-key identity, no pairwise-HMAC migration without owner approval) is respected by the current state. |
| I-004 | Dashboard authentication | Re-check the attached audit's claim that `/login` is plaintext/unsafe against current routes and deployment defaults. If confirmed, fix the smallest security boundary defect and add negative/auth regression tests. Do not infer exposure from a route name alone. | CLOSED (`b1a9e9e`) — the audit's claim does not hold against the current implementation: the dashboard password is an Argon2 PHC, login is lockout-protected, and the mutating route `POST /api/config` requires a valid session (401 otherwise) with `dashboard.require_auth_for_config` defaulting to `true`. `/login` is deliberately exempt from the session middleware because it is the login page. The same change closed the unauthenticated local IPC mutation path, which the row does not mention. Report: `docs/development/POSTURE_CHANGE_FIXES.md`; review `POSTURE_CHANGE_REVIEW.md` (PASS). |

### P1 — enforcement and bounded recovery

| ID | Area | Required work / acceptance evidence | Status |
|---|---|---|---|
| I-010 | XDP map reconcile iterator churn | Revalidate `AyaXdpApplier::reconcile` in `crates/ramshield-enforcement/src/xdp.rs`. If deletion during `get_next_key` can invalidate iteration, collect stale keys in a bounded buffer before deletion; handle `ENOENT` and map churn without loops/skipped live rules. Add a churn regression test and run enforcement tests. | CLOSED (`2868250`) — the reconcile plans first and deletes afterwards from a bounded staged vector, `ENOENT` is handled explicitly, and `reconcile_enumeration_terminates_with_a_stale_key` covers the stale-key enumeration termination. This is exactly what the row asked for. |
| I-011 | Storage eviction vs threat accumulators | Verify cardinality-driven LRU eviction cannot repeatedly erase the attacker's own CUSUM/pulse-wave history and create a threshold bypass. Define bounded behavior under capacity pressure; add adversarial rotating-IP tests before changing policy. | OPEN — no master record closes this, and the defect was not re-verified in this pass. UNVERIFIED whether it exists on current master. |
| I-012 | Host-network Kubernetes capability review | Review `deploy/k8s/daemonset.yaml` capabilities against the exact XDP/capture operations required. Minimize capabilities and document why each retained capability is required; validate manifests and deployment smoke tests. Do not remove capabilities blindly if it breaks enforcement. | OPEN — verified still present: `deploy/k8s/daemonset.yaml` still runs with `hostNetwork: true` and a `NET_ADMIN` capability, and nothing in the master records documents why each retained capability is required. The row's own acceptance ask is unmet. |
| I-013 | SYNPROXY fixture parity | Compare `tests/fixtures/synproxy.nft` with `config.baseline.toml` and actual generator behavior; align ports and prove port 80 coverage if discrepancy remains. | OPEN — verified still present: the fixture matches ports {22, 443} while `config.baseline.toml` declares [22, 53, 80, 443]. Ports 53 and 80 are not in the fixture. |

### P2 — maintenance, reproducibility, and operator correctness

| ID | Area | Required work / acceptance evidence | Status |
|---|---|---|---|
| I-020 | Dashboard quarantine search | Confirm whether `#tableSearch` has an input listener in `src/dashboard/static/index.html`. If absent, connect it to the existing filter function and test filtering by IP, CIDR and module. | OPEN — verified still present: the `#tableSearch` input exists and `filterTable` is defined, but `filterTable` occurs exactly once in the file — at its own definition — and is never bound to an input listener. |
| I-021 | Container build-cache behavior | Inspect `Containerfile` and prove the layer-cache invalidation problem before patching. Keep dependency/build layers cached while ensuring real source changes rebuild the binary; validate with two controlled builds or CI evidence. | NOT APPLICABLE (evidence) — the described layer-cache hazard is not present in the current `Containerfile`: the dependency inputs are copied in an earlier layer than the sources (`COPY Cargo.toml Cargo.lock ./` and `COPY crates ./crates` before `COPY src ./src`), and the build layer additionally busts on touched `.rs` files. The row's "prove the problem before patching" precondition is met in the opposite direction, so there is nothing to patch. |
| I-022 | Systemd portability | Search systemd unit templates (including backlog dispatcher) for developer-specific absolute paths. Replace with package-relative or configurable paths and verify service working-directory/user behavior. | OPEN — verified still present: `systemd/user/ramshield-backlog-dispatcher.service` hard-codes a developer-specific absolute path for both `WorkingDirectory` and `ExecStart`. |
| I-023 | Remove stale repository artifacts | Check whether `src/engine/boot.rs.orig` and root `fix.patch` still exist and whether any references require them. Delete only if present and unneeded; ensure clean build/tests and no packaging references. | OPEN (half done) — `src/engine/boot.rs.orig` was deleted after its content was proven to live in git history (`6efa8f9`); the root `fix.patch` is still tracked on master. |
| I-024 | Protocol HMAC documentation | Recompute the exact signing string from the implementation and tests, then correct `crates/ramshield-protocol/README.md` to match. Add/retain a test or golden vector that locks down key ID, timestamp, payload encoding and signature. Do not copy the attached report's proposed format without source verification. | OPEN — not re-verified in this pass. UNVERIFIED whether the README's signing string matches the implementation today. |
| I-025 | Dependency/documentation drift | Verify `ramshield-types` error dependency split and `BlockReason` documentation, config schema examples, analytics/Welford docs, XDP hardware verification limits, forecast priors/polling claims, and module-level status claims. Update only stale factual claims and clearly identify tests that require live kernel/hardware. | OPEN — not re-verified in this pass. UNVERIFIED. |
| I-026 | Benchmark harness duplication | Inspect `ramwal` benchmark targets for duplicated Criterion harness code. Consolidate only if it reduces maintenance without obscuring benchmark comparability; attach before/after commands/results. | UNVERIFIABLE — the path the row names (`crates/ramshield-ramwal/benches`) does not exist on master; the crate's layout differs, so the finding cannot be located or dismissed as written. |
