# RamShield audit findings backlog — the audit-branch era record

**This file is history, not a ledger.** It is the verbatim record of the
audit-branch remediation passes and the 2026-10-17 audit import. It
describes the branch `audit-branch` and the SHAs that were current then.
None of it describes current master state.

**The current records are three files, and only these three:**

| question | file |
|---|---|
| What is open, and what is the closure criterion for it? | [`docs/development/OPEN_FINDINGS.md`](OPEN_FINDINGS.md) |
| What is proven on which exact SHA? | [`docs/development/OWNER_DECISION_PACKAGE.md`](OWNER_DECISION_PACKAGE.md) — the per-SHA gate ledger |
| What is the status of every backlog row? | [`BACKLOG.md`](../../BACKLOG.md) at the repository root |

**Why this file exists.** The audit-branch material was moved here
verbatim so that `BACKLOG.md` could be re-pointed at `master` without
destroying the record. Nothing was edited in the move: the text below is
the text that was in `BACKLOG.md`.

**What is NOT here.** The status tables for the A-0xx and I-0xx rows are
not here. They stayed in `BACKLOG.md` and were re-stamped, because a row's
status is a statement about the present, not about the past.

**The header and the handoff contract as they stood on 2026-10-17**, before
`BACKLOG.md` was re-pointed at `master` on 2026-10-21. The lines that were
replaced are recorded here so the earlier framing is not lost:

- **Target branch:** `audit-branch`
- **Baseline HEAD:** `70cd373f3c543ea796dd684197535956b49d6b0c`
- **Baseline master:** `821eab72e93e584cd6835d1f95171d5d380daaec`
- **Latest recorded CI:** [run 38081795011](https://github.com/grep999/ramshield/actions/runs/38081795011) — all CI jobs passed on exact HEAD `59d4bc6d68db132223e9817b61ac4c399b5e965e`, including core-only/full-feature/all-feature tests, build/check, strict Clippy, formatting, dependency audit, and C SHM ABI stress. [Backlog Gate run 38081795019](https://github.com/grep999/ramshield/actions/runs/38081795019) also passed on that SHA, including A-031 core matrix, A-033 SHM ABI/TSAN/AArch64, A-016, formatting, whitespace, and evidence manifest.
- **State review (2026-10-12):** A-035 passed exact-SHA local and hosted validation on `0e48c3bc6ce7997d5e57421930715706b9627f71`; the final two-reviewer independent audit remains A-038. A-036 is partial (see its row), and A-037, A-038, and A-039 remain open. Mesh remains explicitly outside this core backlog and is tracked in `docs/development/MESH_FEATURE_PLAN.md`. The mesh prototype's green checks do not change core backlog status or constitute mesh production acceptance.
- **Handoff contract as it stood:** "Every pass records the base SHA, exact result SHA, one bounded selected change, changed paths, regression coverage, commands actually run with exit status, CI run/job links, remaining findings, and next action. Work only on `audit-branch`. Never merge, force-push, rebase, or alter `kiddo_fix`, PR #3, or PR #5 as part of this workflow."

All three superseded SHAs above are ancestors of master. They are not wrong SHAs — they are simply no longer the state of the world.

---

## 2026-10-10 remediation pass — audit-branch

The following source-level fixes were applied directly to `audit-branch`:

- **SHM probe parity:** C reader and Rust writer now both use a four-slot probe window. A coherent occupied C slot with a different key advances to the next probe instead of retrying the same slot 64 times.
- **Test dispatch:** `suite.py load bench` now dispatches the `.sh` benchmark through Bash. `integration_suite.py` maps its named profiles to supported `suite.py` commands and no longer passes profile strings as arguments to individual scripts.
- **Audit helper:** `audit-safety-commitment.py` uses the repository-relative workspace, no longer treats `#` as a dangerous token, and has its shebang at byte zero.
- **Python import:** `random` is imported at module scope in `scripts/subnet_test.py`.
- **SYNPROXY deployment:** added `deploy/sysctl.d/99-ramshield-synproxy.conf`; the systemd service restores `ProtectKernelTunables=true`; runtime SYNPROXY setup verifies required sysctl values and does not attempt privileged writes.
- **Documentation:** removed the duplicate half of `crates/ramshield-analytics/README.md`, corrected the stale `ipnet` claim in `crates/ramshield-types/README.md`, and populated the previously empty `docs/LOCKDOWN_RUNBOOK.md`.
- **Duplicate tree:** removed the nested `rs/` archive; root-level workspace sources remain.

### Findings checked against current source

- Removed obsolete `pp.patch`: it contained an invalid `Store::set_block_state` call, unconditional mesh imports, bare lock unwraps, and an `unsafe { mem::zeroed() }` test harness. These hunks were not applied to the tracked source. The suggested `update_ip` replacement in the report would violate the documented `Store::update_ip` contract, which is for statistics-only mutation; enforcement must retain its block-state-authoritative write path.
- Mesh fields/modules and boot-time mesh construction are already gated behind `feature = "mesh"`; the dependency is optional.
- The enforcement task's `JoinHandle` is selected by the pipeline; unexpected termination returns a fatal pipeline error rather than leaving the daemon silently healthy.
- The ExaBGP FIFO bridge already opens the FIFO with `O_RDWR`, preventing EOF on writer restart.
- The proposed `concurrency_invariants.rs` file and its `mem::zeroed()` test are absent from the tracked tree.

### Acceptance status

These are applied source changes, **not a release sign-off**. The full Rust/C build, formatting, Clippy, zero-unwrap gate, and end-to-end suite must pass on one final immutable SHA before closure. Keep the remaining backlog items open until that evidence is attached.

---

## 2026-10-12 decomposition/hardening pass — audit-branch

Base SHA: `44dfdae`. Result SHA: `cc685f4`. One bounded pass.

Changed paths: `crates/ramshield-config/src/env.rs`, `crates/ramshield-config/src/validate.rs`, `crates/ramshield-enforcement/src/service/enforce.rs`, `src/engine/native.rs`, `src/ipc/server/connection.rs`, `src/ipc/server/handlers.rs` (decomposition, committed earlier via `e65408e`); `crates/ramshield-cgnat/README.md`, `docs/DEPLOYMENT_COMMANDS.md` (this commit).

Regression coverage: existing suite covers the touched paths. New native.rs misalignment assertion in `tpacket_tests`. Checkpoint-barrier scope in `enforce.rs` verified unchanged by diff review — not covered by a new test.

Commands actually run (all exit 0): `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `cargo test --workspace --locked --features full` (all crate test suites pass, 0 failures).

Remaining findings: A-035 passed its recorded exact-SHA gates; its final audit is tracked by A-038. A-036 documentation is complete and awaits exact-SHA gates/review. A-037/A-038/A-039 remain open.

Next action: qualify A-036 on its exact candidate SHA, then continue with A-037 core benchmark baselines.

---

## 2026-10-13 Independent Audit Review Snapshot

- **Audit SHA**: `09d7af6` (current HEAD on audit-branch)
- **Local CI checks**: `cargo fmt --all -- --check` ✅, `cargo clippy --workspace --all-targets --locked -- -D warnings` ✅, `cargo test --workspace --locked --features full` ✅ (0 failures)
- **A-035**: PASS (exact SHA `0e48c3bc6ce7997d5e57421930715706b9627f71` — CI runs 38297139446/38297139440 passed, local checks verified)
- **A-036**: DOCS COMPLETE (CGNAT README, DEPLOYMENT_COMMANDS.md, INDEX.md). Exact‑SHA gates and independent review pending before final acceptance.
- **A-037**: IN PROGRESS (regression budget table added to BENCHMARKS.md; baselines recorded in `docs/development/BENCHMARK_BASELINES.md`. Kernel capture/TPACKET ring and isolated memory behavior not measured; field-day variance too high for threshold).
- **A-038**: OPEN (final independent audit — requires two independent reviewer assessments. My review in progress; second reviewer still needed).
- **A-039**: OPEN (final acceptance report — pending A‑038 closure. Will attach exact SHA, compare‑to‑master, workflow URLs/job conclusions, fmt result, test counts, known limitations, module scores with evidence, prioritized residual‑risk statement.)
- **A-022**: PASS (WAL stress / heavy traffic testing task completed). Test `wal_stress` passes all 3 tests (wal_heavy_traffic_concurrent_appends, wal_heavy_traffic_mixed_operations, wal_heavy_traffic_recovery_after_stress).

**Next steps**: qualify A-036 exact‑SHA gates, finalize A-038 review, secure second reviewer, draft A-039 acceptance report.

---

**Handling rule:** the attached audit is a source of candidate findings, not proof that each finding still exists. Revalidate every item against the selected branch before code changes. Its module scores are provisional and must not be reported as current ratings without reproducing the review.

---

### Audit execution protocol

1. Start with one bounded task from the highest-priority OPEN row. Re-fetch the selected branch's source at task start; older audit text and old patch hunks are not authoritative.
2. Write a failing regression test or a reproducible demonstration before a behavior fix wherever practical.
3. Patch only the task's files plus directly required tests/docs. Avoid cross-task cleanup in the same commit.
4. Run the narrow tests first, then applicable workspace tests, formatting, strict Clippy, C/ABI tests, and deployment validation for changed surfaces. Record commands and real exit codes; do not claim unrun gates.
5. Push/commit to the intended fix branch, wait for GitHub Actions, inspect failed job logs, fix the root cause, and rerun the narrow gate before dependent/full gates.
6. A reviewer different from the fixer examines the actual diff, regression evidence, exact head SHA and CI results. No auto-merge, deploy, force-push, or production mesh enablement.
7. Update status only with evidence. Each pass records base/result SHA, changed paths, test commands/results, workflow links, remaining risks and the next task.

**First candidate:** I-001/I-002 shared-memory lookup correctness, but only after inspecting the exact intended branch and adding regression tests. The branch name `fix` was not present when checked on 2026-10-17; the repository contains several `fix/*` branches. Do not silently substitute one of them for the requested literal branch.

---

## 2026-10-17 orchestration checkpoint

- Audit import commit: `8edc3b5e0eca6d111ddfacb6d25f5d292e464c53` (on `master`; changes only backlog text).
- Task `I-001/I-002`: [Issue #30](https://github.com/grep999/ramshield/issues/30) — SHM reader contention and IPv4 prefix validation, bounded scope and acceptance gates recorded.
- Workflow task: [Issue #31](https://github.com/grep999/ramshield/issues/31) — close the dispatch/evidence gap before introducing any privileged automation.
- Branch check: no ref named exactly `fix` was found; there are multiple `fix/*` refs. The GitHub branch-create request for literal `fix` was rejected with HTTP 422. No code change was made on an alternate branch.
- CI configuration inspected: `.github/workflows/ci.yml` is configured on pushes to `main`, `develop`, `master`, and `audit-branch`; it does not currently name a literal `fix` branch. `.github/workflows/backlog.yml` runs on `audit-branch` pushes and PRs targeting `audit-branch`, `master`, or `main`, and includes SHM ABI/concurrency checks. Neither was dispatched for the new task as no implementation branch/PR exists yet.
- Current status: backlog imported; first bounded task ready as Issue #30; automated dispatch remains BLOCKED pending workflow task #31 and an explicit branch target.


### 2026-10-17 workflow evidence — backlog commit `99310ec84bb5364790e86f6d685a9487f8cf1c2b`

- CI run: [39063742287](https://github.com/grep999/ramshield/actions/runs/39063742287) on this exact SHA. Latest observed state: workflow-lint succeeded; Formatting, Check, Test, Dependency audit, Build, and Clippy were still running at last inspection. Do not infer final CI status from this snapshot.
- no-unwrap run: [39063742273](https://github.com/grep999/ramshield/actions/runs/39063742273) failed on the same SHA. The logged finding is `crates/ramshield-mesh/src/aworset.rs:204`: `.expect("mesh dot counter exhausted; refusing to emit a duplicate CRDT dot")`. This is a mesh-source policy/gate finding, not caused by the backlog-only diff. Triage it on an authorized dedicated mesh/fix branch; do not expand Issue #30 scope to fix it.
- subnet stress: [39063742259](https://github.com/grep999/ramshield/actions/runs/39063742259) was still running at last inspection.
- No runtime patch, workflow dispatch, or PR has been made by this orchestration pass. The first implementation task is Issue #30, pending an explicit valid branch target and worker execution.
