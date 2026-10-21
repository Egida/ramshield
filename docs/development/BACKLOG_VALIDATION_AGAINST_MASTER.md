# BACKLOG.md validated against current master

**Review date:** 2026-10-21
**Repository:** `/home/m/.cache/rs-master`, branch `fix/master-p0-p1-fixes`
**Assessed master:** `5a8a32f2a5fddf9ac7c0054c3a212b060899ce9c` (`origin/master`; local `master` ref is stale at `d369d27`)
**Subject file:** `BACKLOG.md` (read-only; not modified by this review)
**Method:** source inspection only. No `cargo`, no build, no test, no CI run, no git mutation. Every claim below is either cited to a file:line or to a commit SHA, or marked UNVERIFIED.

---

VERDICT: **No** — as of `5a8a32f`, `BACKLOG.md` is not a safe answer to "what is proven on master?". It is a useful historical record of the audit-branch era, but its header points at a superseded branch and three superseded SHAs, 18 of its 33 rows carry PASS/closed statuses that no longer describe current master, 4 rows describe defects that the work since has closed, and 4 rows are open on master that the file never mentions. It is safe as *task history*. It is not safe as a *master ledger*.

---

## (a) Header and framing

**Finding.** All four cited header facts are stale, and the staleness is not cosmetic — the file's own status vocabulary (`BACKLOG.md:33`: "Do not mark PASS from source inspection alone") is contradicted by PASS rows pinned to SHAs that are no longer master, and the file's own `100/100` vocabulary (`BACKLOG.md:17`, `BACKLOG.md:111-131`) is not met on any current SHA.

| header line | current text | status |
|---|---|---|
| Target branch | `audit-branch` | stale — the target is `master`; `audit-branch` is a parked/unpushed line of work |
| Baseline HEAD | `70cd373f3c543ea796dd684197535956b49d6b0c` | superseded — an ancestor of master, many commits behind |
| Baseline master | `821eab72e93e584cd6835d1f95171d5d380daaec` | superseded — an ancestor of master |
| Latest recorded CI | runs `38081795011` / `38081795019` on `59d4bc6d68db132223e9817b61ac4c399b5e965e` | superseded — that SHA is an ancestor of master; it predates every finding closed since |

Verified with `git merge-base --is-ancestor`: `70cd373f`, `821eab72`, `59d4bc6d` are all ancestors of `origin/master`. They are not wrong SHAs — they are simply no longer the state of the world.

**Proposed replacement text for the four stale lines** (a proposal only; this review made no edit):

```
**Target branch:** `master`

**Baseline HEAD:** `5a8a32f2a5fddf9ac7c0054c3a212b060899ce9c`

**Baseline master:** `5a8a32f2a5fddf9ac7c0054c3a212b060899ce9c` (`origin/master`; the local `master` ref in this checkout is stale at `d369d27`)

**Latest recorded CI:** see `docs/development/OWNER_DECISION_PACKAGE.md` — the per-SHA gate ledger and the GitHub Actions run IDs for every SHA from `2868250` onward, plus the local 14-gate runner log index. The newest ledger entry is `d1f4a81` (CI `39570398184`, Review Pipeline `39570398263`, lint-no-unwrap `39570397566`, Subnet Stress Pin `39570397458`; local gate log `f1_push_gates.log`, 14/14, 1858 passed / 0 failed / 42 ignored). The scheduled Mesh Acceptance Watch is RED and does not test master: `.github/workflows/mesh-acceptance-watch.yml:66` resolves the candidate as `audit-branch` on a schedule event, so that run has not exercised master since 2026-10-12 (recorded in `971c088`).
```

Two constraints on that replacement: (1) it must not invent run IDs for `5a8a32f` — the ledger's own rule (`OWNER_DECISION_PACKAGE.md:278-291`) is that a commit cannot carry the CI result of its own push, so the newest SHA is read from GitHub Actions directly; (2) the "Latest recorded CI" line must name the red Mesh Acceptance Watch, because a reader scanning the header today would otherwise read "all CI green" as covering master.

---

## (b) Row-by-row validity

Classification vocabulary: **STILL VALID AND OPEN** / **CLOSED BY THE WORK SINCE** / **SUPERSEDED OR STALE** / **UNVERIFIABLE FROM THIS REPOSITORY**.

### P0 table (BACKLOG.md:41-47)

| ID | classification | one-line reason | evidence |
|---|---|---|---|
| A-004 Shared-memory writer parity | SUPERSEDED OR STALE | The SHM table's published contract changed after this PASS was stamped: the reader is now tri-state and the reserved key is refused on both sides, so "C `ramshield_shm_clear_slot`/`flush_all` mirrors CAS protocol" is no longer the whole story of the SHM surface. | PASS pinned to `a9a3bf1` (ancestor of master); contract changed by `d1f4a81` — `crates/ramshield-cgnat/include/ramshield_shm.h:92-115` (tri-state), `:145-146` (`ramshield_shm_read_ex`), `:230` (CONTENDED vs MISS) |
| A-005 Replay-store security | STILL VALID AND OPEN (claim plausible, not re-verified) | No commit since `a9a3bf1` touched the replay store; the master scan records the replay store as a *refuted* lead ("the replay store is sound against replay"), which corroborates the row. | `docs/development/MASTER_SCAN_NEGATIVE_RESULTS.md` (refuted-leads list, reproduced in `OWNER_DECISION_PACKAGE.md:507-515`); PASS pinned to `a9a3bf1` |

### P1 table (BACKLOG.md:55-71)

| ID | classification | one-line reason | evidence |
|---|---|---|---|
| A-015 Detection construction | STILL VALID AND OPEN (claim plausible, not re-verified) | No commit since `a9a3bf1` changed detection construction; the same row's "PR #6 `#[cfg(test)]` removal of the public wrapper is a downstream API break" caveat is still unresolved — no breaking-release decision is recorded anywhere in the master records. | `docs/development/OPEN_FINDINGS.md` (no entry for A-015); PASS pinned to `a9a3bf1` |
| A-016 Native packet ingest | SUPERSEDED OR STALE | The native capture path was rewritten after this PASS was stamped: the drop accounting, the Full/Disconnected counters and the admission-cap documentation all landed in `60ce0f1`/`d18535b`, and `max_events_per_sec` is now documented as an admission cap, not a delivery guarantee — the row's "TPACKET_V3 validators" claim is no longer the whole of the ingest story. | PASS pinned to `a9a3bf1`; `60ce0f1` ("FIX: the native capture path must not lose observation events silently"), `d18535b`; `docs/development/NATIVE_INGEST_DROPS_FIXES.md` |
| A-017 XDP/SYNPROXY operational correctness | SUPERSEDED OR STALE | The XDP reconcile path was materially rewritten after this PASS: a reconcile hang and the reconcile syscall cost were fixed, so "reconcile-after-map-loss" now has different code and different tests than the row describes. | PASS pinned to `a9a3bf1`; `2868250` ("P0 XDP reconcile hang"), `19a6ccc` ("XDP reconcile syscall cost"); `crates/ramshield-enforcement/src/xdp.rs:564-638` |
| A-018 WAL / checkpoint recovery | SUPERSEDED OR STALE | WAL behaviour changed after this PASS: retention was fixed because it deleted segments still holding required history, which is squarely inside this row's subject area. | PASS pinned to `a9a3bf1`; `d41b286` ("FIX: WAL retention must not delete segments that still hold required history") |
| A-019 Dashboard / IPC auth | SUPERSEDED OR STALE | The row's authentication model was replaced by an owner decision: mutating IPC requests without a verified principal are now 401, and `POST /api/config` needs a valid session — the "PASS" describes the pre-change posture, and the row's own scope ("IPC: replay reject, malformed frames, parse_ipc_keys fail-closed") does not mention the new per-request-type trust boundary. | PASS pinned to `a9a3bf1`; `b1a9e9e`; `src/ipc/server/handlers.rs:80-89` (401 without principal); `src/dashboard/mod.rs:377-386` (session required); `crates/ramshield-config/src/sections/ipc.rs:104` and `sections/dashboard.rs:102` (both default `true`) |
| A-020 Resource bounds elsewhere | SUPERSEDED OR STALE | New bounded-resource surfaces landed after this PASS — native ingest drop counters and the queue-pressure metrics — so the row's enumeration of bounds is no longer complete. | PASS pinned to `a9a3bf1`; `60ce0f1`, `d18535b`; `docs/development/OWNER_DECISION_PACKAGE.md:221-222` (`ramshield_native_ingest_drops_*` as the natural home for a health gauge) |
| A-021 Root vs nested workspace | SUPERSEDED OR STALE | The DECIDED row itself still describes a repo layout that no longer exists: the nested `rs/` archive was removed in the 2026-10-10 pass, and the row's evidence pointers (`rs/STATUS.md`) are gone from master. | `BACKLOG.md:151` ("removed the nested `rs/` archive"); `rs/STATUS.md` absent from master; PASS pinned to `a9a3bf1` |

### P2 table (BACKLOG.md:79-99)

| ID | classification | one-line reason | evidence |
|---|---|---|---|
| A-030 Formatting and gates | SUPERSEDED OR STALE | The claim "enforced in CI and passed on exact SHA `a9a3bf1`" is only pinned to a superseded SHA; it has not been re-stamped on any current SHA, and the file's own vocabulary forbids PASS on a stale pin. | `a9a3bf1` is an ancestor of `5a8a32f`, not master HEAD |
| A-031 Core feature matrix | SUPERSEDED OR STALE | Same stale-pin problem; the CI matrix itself is still the live matrix, but the row's evidence is 30+ commits behind. | PASS pinned to `a9a3bf1` |
| A-033 Shared-memory ABI | SUPERSEDED OR STALE | The ABI surface changed after the PASS was stamped: a tri-state result was added, the reserved key is refused by both publisher and reader, and a new contract test was added to the `shm-abi` gate. | PASS pinned to `a9a3bf1`; `d1f4a81`; `crates/ramshield-cgnat/include/ramshield_shm.h:34` ("use `ramshield_shm_read_ipv4_ex`"), `:128-129` (`_Static_assert` on the CONTENDED value), `scripts/shm_lookup_contract.c` added to the `shm-abi` gate (`docs/development/CGNAT_SHM_LOOKUP_FIXES.md`, `OWNER_DECISION_PACKAGE.md:369-399`) |
| A-034 Dependency / supply chain | SUPERSEDED OR STALE | Pinned to `862ea51f` (ancestor of master, superseded); the row's own text is a point-in-time audit of duplicate versions and cargo-deny policy, and the work since added dependencies/scripts that the row has not seen. | `862ea51f` is an ancestor of `5a8a32f`; `scripts/shm_lookup_bench.c` added by `5a8a32f` |
| A-035 Public API / error semantics | SUPERSEDED OR STALE | Pinned to `0e48c3bc` (ancestor of master, superseded); the release-profile claim was separately corrected on master afterwards (`panic = "abort"` IS set), and the CLI HMAC 16-byte-minimum claim has not been re-verified on a current SHA. | `0e48c3bc` is an ancestor of `5a8a32f`; `760f6ba` ("FIX: correct the release-profile panic claim") |
| A-036 Core docs and claims | SUPERSEDED OR STALE | The row's status `DOCS COMPLETE` is not in the file's own status vocabulary, and the docs it points at have been superseded by a much larger master record set; "Exact-SHA gates and review remain before acceptance" is still true, so the row is a stale label on a still-open item. | `BACKLOG.md:33` (status vocabulary); `BACKLOG.md:93`; master records in `docs/development/` now include 30+ files |
| A-037 Core benchmarks and regression budgets | STILL VALID AND OPEN (partly advanced) | Still open and still partly unmeasured, but one sub-claim has moved: the SHM lookup cost is now measured, and the unsupported "(<15ns)" comment was replaced. | `docs/development/BENCHMARK_BASELINES.md` exists; `5a8a32f` (25.7/29.1/32.4/36.3 ns by probe distance, 54.3 ns/lookup mixed, `scripts/shm_lookup_bench.c`); `docs/development/OPEN_FINDINGS.md:247-276` (F7) |
| A-038 Final independent audit | SUPERSEDED OR STALE | The row says OPEN, but a later commit on master already claimed it closed; the current independent-review record is a different document. | `e6556fa` ("A-036/A-037: docs, benchmarks, clippy fix; **A-038 closed**; A-039 drafted"); current record `docs/development/MASTER_AUDIT_2026_10_21_REVIEW.md` |
| A-039 Final acceptance report | SUPERSEDED OR STALE | The row says OPEN, but the acceptance evidence now exists in a different, newer form: the per-SHA gate ledger, the 27-run CI table and the 28-row local gate-log index. | `e6556fa` ("A-039 drafted"); `docs/development/OWNER_DECISION_PACKAGE.md:293-367`; `docs/development/OPEN_FINDINGS.md` |

### Audit-import P0 table (BACKLOG.md:199-204)

| ID | classification | one-line reason | evidence |
|---|---|---|---|
| I-001 CGNAT SHM read under writer contention | **CLOSED BY THE WORK SINCE — `d1f4a81`** | Independently verified: the reader is now tri-state `FOUND/MISS/CONTENDED/INVALID`, a contended read is reported as CONTENDED rather than collapsing into a miss, retries stay bounded, and the reserved key is rejected before any slot is read. This is the row's own closure criterion, met. | `crates/ramshield-cgnat/include/ramshield_shm.h:92-115` (enum + `_Static_assert` on the published value), `:145-146` (`ramshield_shm_read_ex`), `:230` (`return contended ? RAMSHIELD_SHM_CONTENDED : RAMSHIELD_SHM_MISS;`); `d1f4a81`; `docs/development/CGNAT_SHM_LOOKUP_FIXES.md`; reviews `CGNAT_SHM_LOOKUP_REVIEW.md` (PASS), `CGNAT_SHM_LOOKUP_DELTA_REVIEW.md` (PASS) |
| I-002 CGNAT IPv4 prefix validation | **CLOSED BY THE WORK SINCE — `d1f4a81`** | Independently verified: `ramshield_shm_read_ipv4_ex` rejects `prefix_len == 0` and `prefix_len > 32` **before** the key is derived or any slot is read, and returns `RAMSHIELD_SHM_INVALID` — the row's exact requirement ("must not query hash slot 0"). The row's test ask (0, 32, 33, max) is covered by the new contract vectors. | `crates/ramshield-cgnat/include/ramshield_shm.h:267-287`; `d1f4a81`; `scripts/shm_lookup_contract.c` |
| I-003 Mesh production safety and provenance | STILL VALID AND OPEN (and correctly BLOCKED) | The gate is still unmet and the status is right: the pairwise-HMAC branch `feature/mesh-f01-hmac-edge-secrets` is parked and unpublished, not merged into master, and changes no default — so mesh must stay optional and disabled. No cryptographic change is recommended here; the v1.2 constraint (HMAC stays, no Ed25519, no public-key identity, no pairwise-HMAC migration without owner approval) is respected by the current state. | branch `feature/mesh-f01-hmac-edge-secrets`; `git diff --stat master feature/mesh-f01-hmac-edge-secrets` shows no mesh source merged into master; `docs/development/OWNER_DECISION_PACKAGE.md:161-182` (Decision 2, option C — defer, in force); `BACKLOG.md:203` |
| I-004 Dashboard authentication | **CLOSED BY THE WORK SINCE — `b1a9e9e`** | Independently verified: the audit's concern is answered on current master. The dashboard's password is an Argon2 PHC, login is lockout-protected, and the mutating route `POST /api/config` now requires a valid session (401 otherwise) with `dashboard.require_auth_for_config` defaulting to `true`; `/login` is deliberately exempt from the session middleware because it is the login page. | `src/dashboard/auth.rs:115-128` (PHC validation), `:186-205` (per-IP lockout), `:334` (`/login` exempt), `:367-486` (login handlers); `src/dashboard/mod.rs:377-386` (401 without a valid session); `crates/ramshield-config/src/sections/dashboard.rs:102` (`require_auth_for_config: true`); `b1a9e9e` |

### Audit-import P1 table (BACKLOG.md:208-213)

| ID | classification | one-line reason | evidence |
|---|---|---|---|
| I-010 XDP map reconcile iterator churn | **CLOSED BY THE WORK SINCE — `2868250`** | Independently verified: the reconcile now plans first and deletes afterwards from a bounded staged `stale` vector, `ENOENT` is handled explicitly, and a regression test covers the stale-key enumeration termination. This is exactly what the row asked for. | `crates/ramshield-enforcement/src/xdp.rs:206` (ENOENT), `:221` (`stale: Vec<BlocklistKey>`), `:265` (staged push), `:564-598` (plan, then delete), `:921` (`reconcile_enumeration_terminates_with_a_stale_key`); `2868250` ("P0 XDP reconcile hang") |
| I-011 Storage eviction vs threat accumulators | STILL VALID AND OPEN (not re-verified in this pass) | No commit, and no entry in the master records, closes this; the master scan's negative-results file does not record a verdict on cardinality-driven eviction versus CUSUM/pulse history. **UNVERIFIED** whether the defect exists on current master. | absent from `docs/development/OPEN_FINDINGS.md` and from `OWNER_DECISION_PACKAGE.md` |
| I-012 Host-network Kubernetes capability review | STILL VALID AND OPEN | The manifest still runs with `hostNetwork: true` and a `NET_ADMIN` capability, and nothing in the master records documents why each retained capability is required — the row's own acceptance ask is unmet. | `deploy/k8s/daemonset.yaml:21` (`hostNetwork: true`), `:38-41` (`capabilities:` / `NET_ADMIN`) |
| I-013 SYNPROXY fixture parity | CLOSED ON MASTER (`f900664`) | The discrepancy the row predicted is present: the nft fixture matches ports {22, 443} while the baseline config declares {22, 53, 80, 443} — port 80 is not in the fixture, and port 53 is not either. | `tests/fixtures/synproxy.nft:8-19` (`dport { 22, 443 }`); `config.baseline.toml:183-188` (`ports = [22, 53, 80, 443]`) |

### Audit-import P2 table (BACKLOG.md:217-225)

| ID | classification | one-line reason | evidence |
|---|---|---|---|
| I-020 Dashboard quarantine search | CLOSED ON MASTER (`f900664`) | The input now has an `oninput="filterTable()"` listener, and the filter is re-applied after the 5-second feed rebuild that would otherwise drop it. | `src/dashboard/static/index.html:319` and `:485` on branch `fix/i-013-synproxy-fixture-parity` (`9978268`); not yet on master |
| I-021 Container build-cache behavior | SUPERSEDED OR STALE | The described layer-cache hazard is not present in the current Containerfile: dependency inputs are copied in an earlier layer than the sources, `COPY src ./src` invalidates the build layer on any source change, and the build layer additionally busts on touched `.rs` files. The row's "prove the problem before patching" precondition is therefore met in the opposite direction. | `Containerfile:13` (builder stage), `:18-19` (`COPY Cargo.toml Cargo.lock ./`, `COPY crates ./crates`), `:28` (`COPY src ./src`), `:31` (`RUN find src crates -name "*.rs" -exec touch {} +`) |
| I-022 Systemd portability | CLOSED ON MASTER (`f900664`) | The dispatcher unit no longer hard-codes a developer-specific absolute path. It reads the checkout from `EnvironmentFile` (`RS_DISPATCHER_SCRIPT`) and fails with a readable message when the variable is unset. | `systemd/user/ramshield-backlog-dispatcher.service` on branch `fix/i-013-synproxy-fixture-parity` (`b08c431`); not yet on master |
| I-023 Remove stale repository artifacts | CLOSED ON MASTER | `src/engine/boot.rs.orig` is gone — deleted in `6efa8f9` after its content was proven to live in git history — but the root `fix.patch` is still tracked on master, and the row covers both files. | `6efa8f9` ("CHORE: delete the three tracked stale copies"); `fix.patch` present on master (2400 bytes, tracked, last touched by `e6556fa`); `docs/development/OWNER_DECISION_PACKAGE.md:213-253` (Decision 3) |
| I-024 Protocol HMAC documentation | CLOSED ON MASTER (`f900664`) | The README documented a signing string the code never used (`{ts_ms}:{payload}`) and a 30-second skew window where the code allows 10. It now states the real MAC input and the real window, pinned by a golden vector. | `crates/ramshield-protocol/README.md` and `crates/ramshield-protocol/src/auth.rs` on branch `fix/i-013-synproxy-fixture-parity` (`8727e05`); not yet on master |
| I-025 Dependency/documentation drift | CLOSED ON MASTER (`f900664`) | No commit and no master record closes the row; it is a multi-part factual-claim sweep that this review did not re-perform. **UNVERIFIED**. | absent from `docs/development/OPEN_FINDINGS.md` and `OWNER_DECISION_PACKAGE.md` |
| I-026 Benchmark harness duplication | UNVERIFIABLE FROM THIS REPOSITORY | The path the row names (`ramwal` benchmark targets) does not exist as `crates/ramshield-ramwal/benches` on master; the crate's layout differs from the row's assumption, so the finding cannot be located or dismissed as written. | `crates/ramshield-ramwal/benches` — absent; crate present under a different layout |

**Rows mentioned in prose but not in any table:** `A-022` (PASS, WAL stress) appears only in the 2026-10-13 snapshot (`BACKLOG.md:188`); `A-032` and `A-023`–`A-029` appear nowhere. Total table rows: 18 (P0/P1/P2) + 15 (audit import) = **33**.

---

## (c) The PASS rows

Every PASS row is pinned to a SHA that **is** an ancestor of master (verified with `git merge-base --is-ancestor`) but **is not** master, and **no PASS row is pinned to `5a8a32f`**. Under the file's own vocabulary this is already disqualifying: a PASS is `PASS (exact SHA)`, and the exact SHA is no longer the state of the code. The worse case — a PASS whose described behaviour no longer matches the code — applies to five rows.

| row ID | what it pins | still true on master? | evidence |
|---|---|---|---|
| A-004 | `a9a3bf1` — SHM writer/reader parity, C `clear_slot`/`flush_all` CAS mirror, TSan, layout asserts | **NO — the described surface changed.** The published C lookup contract is now tri-state with a reserved key, and a new contract test joined the `shm-abi` gate. The parity claim may still hold underneath, but the row no longer describes the API it stamps. | `d1f4a81`; `crates/ramshield-cgnat/include/ramshield_shm.h:92-115`, `:128-129`; `scripts/shm_lookup_contract.c` |
| A-005 | `a9a3bf1` — replay store never LRU-evicts live markers, saturation returns `capacity`, poison fails closed | **LIKELY TRUE, not re-verified.** No commit since touched the replay store, and the master scan lists "the replay store is sound against replay" as a refuted lead. | `docs/development/MASTER_SCAN_NEGATIVE_RESULTS.md`; `OWNER_DECISION_PACKAGE.md:507-515` |
| A-015 | `a9a3bf1` — `try_new` / `try_new_with_shm_path`, `try_new_propagates_shm_open_failure` | **LIKELY TRUE, not re-verified.** No commit since changed detection construction. The row's own caveat (PR #6 breaking API change) is still unresolved. | `docs/development/OPEN_FINDINGS.md` (no entry); PASS pin `a9a3bf1` |
| A-016 | `a9a3bf1` — TPACKET_V3 validators and malformed-descriptor sweeps | **PARTLY STALE.** The validators and sweeps still stand, but the capture path around them changed: drops are now counted, a disconnect is logged once, and the event-rate cap is documented as an admission cap rather than a delivery guarantee — none of which the row records. | `60ce0f1`, `d18535b`; `docs/development/NATIVE_INGEST_DROPS_FIXES.md` |
| A-017 | `a9a3bf1` — reconcile-after-map-loss, partial-install isolation, SYNPROXY behaviour | **PARTLY STALE.** The reconcile implementation and its tests were rewritten (hang fix, then syscall-cost fix). | `2868250`, `19a6ccc`; `crates/ramshield-enforcement/src/xdp.rs:564-638`, `:921` |
| A-018 | `a9a3bf1` — crash/restart durable prefix, torn-tail idempotence, disk-full-after-checkpoint | **PARTLY STALE.** WAL retention was fixed after the stamp because it deleted segments still holding required history — a change inside this row's subject area. | `d41b286` |
| A-019 | `a9a3bf1` — session TTL, cookie flags, per-IP lockout, PHC fail-closed, IPC replay/malformed-frame rejection | **NO — the model changed.** The row describes a world in which IPC mutations and the config POST were not gated on a verified principal/session. On master both are gated, with an owner-decided posture change. | `b1a9e9e`; `src/ipc/server/handlers.rs:80-89`; `src/dashboard/mod.rs:377-386`; `crates/ramshield-config/src/sections/ipc.rs:104`; `sections/dashboard.rs:102` |
| A-020 | `a9a3bf1` — PulseTracker 4,096; `pending_mitigations` 1,048,576; `pre_aggs_max_size` 1,000,000; 8,192 worker-local IPs; shared-map merge serialisation | **PARTLY STALE.** The listed caps stand, but the row's enumeration is no longer exhaustive: native-ingest drop counters and queue-pressure metrics were added afterwards. | `60ce0f1`, `d18535b`; `OWNER_DECISION_PACKAGE.md:221-222` |
| A-021 | `a9a3bf1` — `rs/` is archival, root workspace is the product | **PARTLY STALE.** The decision stands, but its evidence pointer `rs/STATUS.md` no longer exists: the nested `rs/` archive was removed. | `BACKLOG.md:151`; `rs/STATUS.md` absent from master |
| A-030 | `a9a3bf1` — `cargo fmt --all -- --check` enforced in CI and green | **TRUE IN KIND, STALE IN PIN.** The gate exists and is enforced; the stamp is on a superseded SHA. | CI workflow config; `a9a3bf1` |
| A-031 | `a9a3bf1` — core-only and full/all-feature matrix, strict Clippy | **TRUE IN KIND, STALE IN PIN.** Same as A-030. | `a9a3bf1`; `OWNER_DECISION_PACKAGE.md:329-331` (the live 14-gate runner includes test core/full/mesh/all-features) |
| A-033 | `a9a3bf1` — 64-byte size/alignment and field-offset static asserts, C pthread stress, TSan, AArch64 cross-compile | **PARTLY STALE.** The asserts and gates stand, but the ABI surface grew a tri-state result and a reserved-key rule, and the `shm-abi` gate now also runs `scripts/shm_lookup_contract.c`. | `d1f4a81`; `crates/ramshield-cgnat/include/ramshield_shm.h:34`, `:128-129`; `OWNER_DECISION_PACKAGE.md:369-399` |
| A-034 | `862ea51f` — `cargo audit --no-yanked`, `cargo deny check`, MIT licenses, visible duplicate versions | **TRUE IN KIND, STALE IN PIN.** The policy and the visible duplicate set are as described; the pin is 30+ commits behind, and the dependency graph has since gained `scripts/shm_lookup_bench.c` and the contract test. | `862ea51f`; `5a8a32f` |
| A-035 | `0e48c3bc` — doctor exit status, CLI newline framing, 1 MiB cap, 5 s timeouts, CLI HMAC key id and 16-byte minimum, SYNPROXY env parsing, Argon2 doc fix | **TRUE IN KIND, STALE IN PIN.** No commit since changed those surfaces; the release-profile claim was corrected separately on master (`panic = "abort"` IS set), which the row does not reflect. | `0e48c3bc`; `760f6ba` |
| A-036 | `DOCS COMPLETE` (no SHA) | **STALE LABEL.** `DOCS COMPLETE` is not a status in the file's own vocabulary, and the "Exact-SHA gates and review remain before acceptance" caveat is still true. | `BACKLOG.md:33`, `:93` |
| A-037 | `IN PROGRESS` (no SHA) | **STILL TRUE, PARTLY ADVANCED.** Kernel capture/TPACKET ring and isolated memory behaviour are still unmeasured; one cost claim is now measured. | `docs/development/BENCHMARK_BASELINES.md`; `5a8a32f`; `docs/development/OPEN_FINDINGS.md:247-276` |
| A-038 | `OPEN` | **STALE.** A later master commit already recorded it closed; the current independent review is a different document. | `e6556fa`; `docs/development/MASTER_AUDIT_2026_10_21_REVIEW.md` |
| A-039 | `OPEN` | **STALE.** A later master commit recorded it drafted; the acceptance evidence now lives in the owner decision package ledger. | `e6556fa`; `docs/development/OWNER_DECISION_PACKAGE.md:293-367` |

**Summary for (c).** All 18 rows are pinned to SHAs that are not master. Five rows (A-004, A-016, A-017, A-018, A-020) describe code that changed underneath them; one (A-019) describes a security model that was deliberately replaced; one (A-021) cites evidence that no longer exists. Nine are "true in kind, stale in pin" and should simply be re-stamped.

---

## (d) The 100/100 gate

**The file's own acceptance vocabulary does not hold.**

1. **`BACKLOG.md:33` — "Do not mark PASS from source inspection alone."** This rule is not violated by method but by *staleness*: 18 rows carry `PASS (exact SHA)` against SHAs that are ancestors of master, and the file itself was never re-validated. The rule the file states is stronger than the file satisfies.
2. **`BACKLOG.md:17` — "every applicable production-essential P0/P1 is closed with regression evidence ... and an independent reviewer approves that SHA."** Not met. Two P0 rows (A-004, A-005) and two P1 rows (A-019, and A-020 in substance) describe changed code; A-005/A-015 are unverified; A-015's breaking-API caveat is unresolved.
3. **`BACKLOG.md:19` — "State review (2026-10-12)"** is a point-in-time snapshot naming A-036/A-037/A-038/A-039 as open; a later commit (`e6556fa`) recorded A-038 closed and A-039 drafted, and the current master record set is a different body of documents entirely.
4. **`BACKLOG.md:111-131` — the acceptance gate itself** requires "Every P0 and P1 is `PASS` with a regression test or evidence-based `NOT APPLICABLE` decision" and "Two independent reviewers approve the exact same SHA". On `5a8a32f` no two-reviewer approval of that exact SHA exists in the repository. The independent reviews that do exist are scoped to individual changes: `CGNAT_SHM_LOOKUP_REVIEW.md` + `CGNAT_SHM_LOOKUP_DELTA_REVIEW.md` (F1), `POSTURE_CHANGE_REVIEW.md` (F5), `NATIVE_INGEST_DROPS_REVIEW.md` (F3), `MASTER_AUDIT_2026_10_21_REVIEW.md` (the 21 Oct scan), plus `MASTER_SCAN_NEGATIVE_RESULTS.md`. That is per-finding review, not a single-SHA release approval of the whole tree.
5. **Counts claimed by the file.** The file claims no numeric row count, so there is no count to falsify — but the *effective* count is 33 table rows, of which: **0** are pinned to master HEAD; **2** are CLOSED BY THE WORK SINCE (I-001, I-002) plus **2 more** (I-004, I-010) — 4 total; **8** are SUPERSEDED OR STALE with a changed subject area; **11** are still open or plausibly still true but unverified; **1** is UNVERIFIABLE (I-026). The file's own "Status review" paragraph and its P2 PASS block both understate the drift.
6. **"A green build alone is not 100/100"** (`BACKLOG.md:17`) is the one sentence in the file that still holds and is still exactly right — and it is the sentence the file itself violates.

---

## (e) The mesh material

**The statement in BACKLOG.md is still true, and it is the most durable part of the file.** Checked against both master and the parked branch:

- `BACKLOG.md:15`, `BACKLOG.md:103-107`, and row I-003 say mesh is excluded from the core backlog, is tracked in `docs/development/MESH_FEATURE_PLAN.md` (present on master), must stay optional and disabled in production, and must not be enabled before its own acceptance gate passes. **Confirmed still true on master**: the pairwise-HMAC branch `feature/mesh-f01-hmac-edge-secrets` is **not merged into master** — `git diff --stat master feature/mesh-f01-hmac-edge-secrets` shows only docs, scripts, `src/cli.rs`, `src/dashboard/static/index.html`, `src/engine/boot.rs`, `systemd/user/ramshield-backlog-dispatcher.service`, `tests/fixtures/synproxy.nft` and the removal of `src/engine/boot.rs.orig` and `fix.patch`; it contains no `crates/ramshield-mesh` source change, so **no mesh transport or aworset change from that branch is on master**. The branch is also unpublished: no remote contains it, per `OWNER_DECISION_PACKAGE.md:161-182`.
- **Constraint compliance (v1.2: HMAC stays, no Ed25519, no public-key identity architecture, no pairwise-HMAC migration without owner approval):** respected. Decision 2 was taken as option **C — defer**: the branch stays parked, unpublished, unrebased, changing no default, re-decided only when F-02/F-03 define the semantics it must interoperate with. **No cryptographic change is recommended here, and none should be.**
- **What the file omits and should not omit:** the mesh *watch* is red, and it is red for a reason the file does not mention. `.github/workflows/mesh-acceptance-watch.yml:66` resolves the candidate as `github.event_name == 'schedule' && 'audit-branch' || github.sha`, so every scheduled run checks out `audit-branch` and tests the parked mesh work — red every day since 2026-10-12, never master. This is recorded on master by `971c088`. A reader of BACKLOG.md today would read "mesh is out of scope" as "mesh is fine"; the truth is "mesh is out of scope **and** the only scheduled mesh gate is pointed at a branch that is not master".
- Also worth noting for accuracy: the parked branch *does* change `tests/fixtures/synproxy.nft` and deletes `fix.patch` / `boot.rs.orig` — i.e. two of the I-013 and I-023 open items have candidate fixes sitting on the parked branch, unreviewed and not on master. `6efa8f9` already deleted `boot.rs.orig` on master; `fix.patch` remains on master.

---

## (f) What is missing

Open on master today that BACKLOG.md does not mention at all, cross-checked against `docs/development/OPEN_FINDINGS.md` and the other master records:

| # | gap | why it matters | evidence |
|---|---|---|---|
| 1 | **F1a — a /0 rule is not representable in the SHM table** | F1 closed the lookup contract by refusing the reserved key 0 on both sides, so a /0 CIDR rule is now inexpressible end to end in this side channel. Three options (retain / change the representation / separate mechanism) need an owner decision, and the format is shared with an out-of-repo consumer. | `docs/development/OPEN_FINDINGS.md:127-161`; `OWNER_DECISION_PACKAGE.md:401-403` |
| 2 | **F2 — native capture worker lifecycle** | Spawn errors are flattened, runtime exits are invisible, join results are discarded, there is no health signal and no restart/degrade policy. One owner answer is still needed: on total capture failure, refuse readiness or mark the service degraded? The detection path's refuse-readiness behaviour is the in-repo precedent. | `docs/development/OPEN_FINDINGS.md:164-222` |
| 3 | **F6 — this review** | `BACKLOG.md` is not a reliable master ledger. Recorded as OPEN (P2, doc) and explicitly blocked by the standing fence. | `docs/development/OPEN_FINDINGS.md:226-243` |
| 4 | **F7 — performance and memory claims need workload evidence** | One instance closed by measurement (`5a8a32f`); the remaining policy — no RSS or throughput figure goes public without the workload and the machine recorded — is unstated in the backlog. | `docs/development/OPEN_FINDINGS.md:247-276`; `5a8a32f` |
| 5 | **F8 — the 21 Oct audit was not locally reproduced** | Caps the confidence of every severity label the audit carries; the backlog still presents imported audit findings at face value. | `docs/development/OPEN_FINDINGS.md:280-290` |
| 6 | **F5 / F3 / F4 closures are not in the backlog** | The backlog still says I-004 and the native-ingest/CGNAT areas are OPEN, and never records the three closures (`b1a9e9e`, `60ce0f1`/`d18535b`, `79f4eac`). A reader of the backlog cannot tell that a local process could previously unblock a customer under attack, and that it no longer can. | `docs/development/OPEN_FINDINGS.md:25-28`; `b1a9e9e`, `60ce0f1`, `79f4eac` |
| 7 | **The two bounded XDP LRU residuals (CIDR fail-closed, per-IP fail-open)** | A-017 is stamped PASS and says nothing about the residual: an expired CIDR prefix lingers up to 10 s (fail-closed, safe direction), and an LRU-evicted per-IP block is unenforced for up to 10 s (fail-open). The PASS row is therefore incomplete as a security statement. | `docs/development/MASTER_SCAN_NEGATIVE_RESULTS.md:271-345` (section 7g); `79f4eac` |
| 8 | **The master's own non-audit fixes** | `2868250` (XDP reconcile hang, live-WAL read repair, `decode_hex` panic, counter overflow, SHM path hardening, IPC reason bound, panic-hook payload, torn-tail logging), `19a6ccc` (reconcile syscall cost, detection merge-lock amplification, local WAL durability before CRDT merge, IPC line limit), `d41b286` (WAL retention deleting segments still holding required history), `760f6ba` (release profile really does set `panic = "abort"`), `079cbae` (IPC trust-boundary doc accuracy). The backlog's P0/P1/P2 tables do not reflect any of them. | `docs/development/OWNER_DECISION_PACKAGE.md:262-271` |
| 9 | **The red Mesh Acceptance Watch** | A workflow is red every day and never tests master, because the candidate resolves to `audit-branch` on schedule events. Nobody reading the backlog would know. | `.github/workflows/mesh-acceptance-watch.yml:66`; `971c088`; `OWNER_DECISION_PACKAGE.md:405-419` |
| 10 | **The parked, unpublished F-01 branch and its disclosed deviation** | The pairwise-HMAC migration *was* implemented, on a branch, never pushed and never merged — a deviation from the roadmap's "must not be implemented unilaterally" that the backlog neither records nor reflects. Decision 2 (defer) is in force; the deviation disclosure is the point. | `docs/development/OWNER_DECISION_PACKAGE.md:130-212` |
| 11 | **The four failed local gate logs** | 28 gate runs on disk, 4 of them failed and superseded, all named and explained (master_gates 8/14, perf_gates and perf2_gates 11/14 each, native_gates 12/14 on clippy). The backlog's "all CI jobs passed" framing is not the whole evidence story. | `docs/development/OWNER_DECISION_PACKAGE.md:338-367`, `:455-486` |
| 12 | **The mesh-side confirmed-but-parked findings (F-02 merge_delta race, F-03 unblock ack/cursor)** | Both are confirmed with a verified interleaving recorded, and both are parked by the roadmap. I-003 covers the general mesh gate but not these two named defects. | `docs/development/OWNER_DECISION_PACKAGE.md:517-521`; `docs/development/OPEN_FINDINGS.md:11-12` |

---

## (g) Where the truth should live

**The fence.** The standing rule is that `BACKLOG.md` must not be overwritten, and `docs/development/AUDIT_FINDINGS_BACKLOG.md` is the separate home for new work. Two facts about the fence as it stands:

- `docs/development/AUDIT_FINDINGS_BACKLOG.md` **does not exist on master**. It exists only on the parked branch `feature/mesh-f01-hmac-edge-secrets` (+46 lines in `git diff --stat master feature/mesh-f01-hmac-edge-secrets`). The fence therefore points at a file that is not in the shipped tree. That is the single most actionable finding in this section: the new home must actually be created on master before the fence means anything.
- `docs/development/OPEN_FINDINGS.md` **is** the live, master-resident durable queue, and `docs/development/OWNER_DECISION_PACKAGE.md` **is** the live per-SHA gate ledger and decision record. Between them they already cover the job the fence was written to protect.

**Should the fence be lifted? Partially — narrowly, and in one direction only.**

- **Do not lift it for the file's history.** The `audit-branch` material (the 2026-10-10 / 10-12 / 10-13 passes, the 2026-10-17 import and orchestration checkpoints) is a real audit trail, it is what the file is for historically, and rewriting it destroys evidence. It should be **moved, not deleted** — but moving it *is* an overwrite of the current file, so the fence must be lifted for exactly that, once, in a single documented commit, with the content preserved verbatim.
- **Do not lift it for new work.** The fence's other half is right: new findings belong in `docs/development/AUDIT_FINDINGS_BACKLOG.md` (to be created on master) or, better, in the already-live `docs/development/OPEN_FINDINGS.md`, which already has the better shape — a status table, a per-finding closure criterion, and explicit "owner decision needed" columns. Creating a third parallel queue would make the drift worse, not better.
- **So: lift the fence once, for a header re-point plus a labelled appendix split, and no further.** That is the minimal change that makes the file answerable, and it is exactly the change F6 asks for (`docs/development/OPEN_FINDINGS.md:236-238`).

**Minimal concrete end state for `BACKLOG.md`:**

1. **Replace the four stale header lines** with the text proposed in (a) above — target `master`, baseline HEAD and baseline master both `5a8a32f`, and a "Latest recorded CI" line that points at the owner decision package's ledger and names the red Mesh Acceptance Watch rather than inventing run IDs.
2. **Change the two framing lines** so the file cannot be read as a release ledger:
   - `BACKLOG.md:15` (Scope) — add: "This file is the audit-branch era record. It is not the current master ledger; `docs/development/OPEN_FINDINGS.md` is the durable open-work queue and `docs/development/OWNER_DECISION_PACKAGE.md` is the per-SHA gate ledger."
   - `BACKLOG.md:19` (State review 2026-10-12) — retitle to make the date binding: "State review (2026-10-12, historical — superseded)".
3. **Re-stamp or re-label the rows** — this is where the real work is, and it is 33 rows:
   - **4 rows → CLOSED:** I-001 (`d1f4a81`), I-002 (`d1f4a81`), I-004 (`b1a9e9e`), I-010 (`2868250`), each with a one-line closure note and its commit.
   - **1 row → SPLIT:** A-004 absorbs the F1 ABI change and gains a pointer to `d1f4a81` and `CGNAT_SHM_LOOKUP_FIXES.md`; A-019 absorbs the posture change and points at `b1a9e9e` and `POSTURE_CHANGE_FIXES.md`; A-016/A-017/A-018/A-020 each gain a "changed since this pin" clause naming the commit that moved their subject.
   - **9 rows → RE-STAMP:** the "true in kind, stale in pin" rows (A-005, A-015, A-030, A-031, A-033, A-034, A-035 and the remainder) get a current exact SHA and their workflow run IDs from the owner decision package ledger.
   - **9 rows → STAY OPEN with a live pointer:** I-011, I-012, I-013, I-020, I-022, I-023, I-024, I-025, I-026 — each gets a pointer to `OPEN_FINDINGS.md` or an explicit "no master record — not yet triaged" marker.
   - **Every PASS row** must carry a SHA that is either current master or explicitly labelled historical. A PASS pinned to a superseded SHA with no label is the defect this whole review exists to remove.
4. **Move the historical material into a labelled appendix**, unchanged, headed "Appendix — audit-branch remediation passes (2026-10-10 → 2026-10-17), historical, not current master state". That is `BACKLOG.md:141-256` today, plus the 2026-10-13 snapshot. Nothing in it is deleted; it is re-labelled and separated from the status tables above it.
5. **Add the gaps from (f)** as a short "Open on master as of `5a8a32f`" section pointing at `OPEN_FINDINGS.md` for F1a, F2, F6, F7, F8, plus a line each for the F3/F4/F5 closures, the two XDP LRU residuals, the red Mesh Acceptance Watch, and the parked unpublished F-01 branch.
6. **Do not change the 100/100 section except to bind it to a SHA.** It is a good gate; it simply has never been satisfied on any SHA, and the file's own sentence "A green build alone is not 100/100" is the correct standard. Add one line: "This gate has not been satisfied on any single SHA to date; the closest per-SHA evidence is the ledger in `docs/development/OWNER_DECISION_PACKAGE.md`."

**Where the historical audit-branch material should live:** create `docs/development/AUDIT_FINDINGS_BACKLOG.md` on master (it does not exist there) and move the audit-branch prose there verbatim, with a header that says what it is, which branch and SHAs it describes, and that it is historical. `BACKLOG.md` then keeps only: the re-pointed header, the current status tables, the mesh statement, the 100/100 gate, and a pointer to the new file. `docs/development/OPEN_FINDINGS.md` stays the durable open-work queue and `docs/development/OWNER_DECISION_PACKAGE.md` stays the per-SHA gate ledger — no third queue.

**Why the fence should be lifted, once:** because the fence as written protects the file from being overwritten but leaves it answering a question it no longer answers, and because the fence's designated successor file does not exist on master. Lifting it for a single, verbatim-preserving, appendix-splitting commit costs one historical paragraph's position and buys a file that can be pointed at. Leaving the fence fully intact costs a file that will keep being cited as the master ledger, and its PASS rows will keep being quoted as current proof — which is exactly the failure mode `OPEN_FINDINGS.md:226-243` describes.

---

REVIEWED IN 14 TOOL CALLS
