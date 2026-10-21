# Independent review of `ramshield_master_audit_2026-10-21.md`

**Audited SHA:** `e5caac123f0976091d37d866c51428a31eee88e4` (`master`)
**Reviewed repo:** `/home/m/.cache/rs-master` (HEAD == `e5caac1`, branch `fix/master-p0-p1-fixes`)
**Method:** read-only source inspection at the exact audited SHA (`git show e5caac1:<path>` where the working tree differs), plus the *uncommitted* working-tree change, which is documented separately where it bears on a finding.
**No tests, clippy, C SHM gate, live XDP or crash-injection runs were performed** — the review is source-level, exactly like the audit it reviews.

Every audit claim was treated as UNVERIFIED until checked against source.

---

## 1. Summary table

| # | Finding | Audit severity | My verdict | My severity | Actionable now? |
|---|---------|----------------|------------|-------------|-----------------|
| F1 | CGNAT SHM lookup conflates contention with a miss | P1 | **CONFIRMED (contract defect; no in-repo production caller)** | P2 in-repo / P1 for the published C ABI | Yes — issue #30, mechanical |
| F2 | Native capture workers can stop without durable health signal or restart | P1 | **CONFIRMED (lifecycle/observability gap)** | P2 | Yes — mechanical |
| F3 | Native ingest drops events on a full channel, uncounted at the producer | P1/P2 | **CONFIRMED (accuracy of the observation); severity overstated** | P2 | Yes — mechanical |
| F4 | XDP LRU eviction is a real protection trade-off | P1 | **PARTIAL — the mechanism is confirmed; the audit misreads which sentence it critiques, and overclaims the "safe direction" wording** | P3 (doc wording) | Doc wording fix only |
| F5 | Dashboard intentionally unauthenticated when no password hash | P1 | **CONFIRMED on the audited SHA; addressed on the uncommitted working tree** | P1 on `e5caac1` / mitigated in working tree | Yes, on the working tree — already done |
| F6 | BACKLOG.md is not a reliable master release ledger | P2 | **CONFIRMED** | P2 | Yes — doc fix |
| F7 | Performance/memory claims still require workload evidence | P2 | **CONFIRMED (method caveat); PR #34 state UNVERIFIED** | P2 (process) | Yes — process item |
| F8 | No local reproduction in the audit pass | P2 | **CONFIRMED (accurate self-disclosure)** | n/a — method caveat | No — but it caps the weight of the whole audit |

---

## 2. Per-finding detail

### F1 — CGNAT shared-memory lookup conflates contention with a miss

**STATUS: CONFIRMED** (both sub-claims) — with the important qualifier that the auditor himself states: no in-repository *production* caller was established, and I confirm that too.

**EVIDENCE — contention vs. miss**

`crates/ramshield-cgnat/include/ramshield_shm.h:97-99`
```c
for (unsigned attempt = 0; attempt < 64; ++attempt) {
    uint32_t before = atomic_load_explicit(&entry->seq, memory_order_acquire);
    if (before & 1u) continue;
```
`…/ramshield_shm.h:112-113`
```c
uint32_t after = atomic_load_explicit(&entry->seq, memory_order_acquire);
if (before != after || (after & 1u)) continue;
```
`…/ramshield_shm.h:128-137`
```c
            // Empty terminates the probe: writers never place a rule beyond
            // the first empty candidate. Expired entries are skipped because
            // an older rule may have expired while a later live collision
            // remains in the bounded probe window.
            if (client_hash == 0) break;
            /* Coherent occupied slot with another key: advance probe now. */
            break;
        }
    }
    return false;
}
```

Decisive reading: the inner 64-attempt loop is exited **only** via `break` (an empty slot, or a coherently-occupied slot with a different key) or via `return true` (a hit). The `continue` paths are exactly the contended ones (odd `seq`, or `seq` changed under the read). If a writer holds the seqlock odd, or changes it on every attempt, the loop runs 64 times, falls out of the inner loop, and the outer probe loop advances to the next probe slot; after the whole probe window is exhausted the function reaches `return false` (`…/ramshield_shm.h:137`). The API therefore returns the same `false` for "no such rule" and "could not be read coherently". The audit's wording ("the code then proceeds through the probe window and ultimately returns `false`") is precise and matches the source. **There is no tri-state result type** — the return type is plain `bool` (`…/ramshield_shm.h:85`).

**EVIDENCE — invalid prefix routed into the lookup**

`…/ramshield_shm.h:25-32`
```c
ramshield_ipv4_subnet_key(uint32_t client_ip, uint8_t prefix_len)
{
    if (prefix_len > 32) {
        return 0;  // reject invalid prefix; caller must treat as policy error
```
`…/ramshield_shm.h:140-151`
```c
static inline bool
ramshield_shm_read_ipv4(const RamshieldShmRuleEntry *table,
                        uint32_t client_ip,
                        uint8_t prefix_len,
                        uint64_t now_ms,
                        RamshieldShmRuleSnapshot *out)
{
    return ramshield_shm_read(
        table,
        ramshield_ipv4_subnet_key(client_ip, prefix_len),
        out);
```
The comment says "caller must treat as policy error", but the caller does not: it passes the `0` key straight into the table lookup. Key `0` is the same sentinel the reader uses for an empty slot (`client_hash == 0`, `…/ramshield_shm.h:132`), and an empty slot has `expires_at_ms == 0`, so the lookup degenerates to a silent miss. Confirmed exactly as claimed.

**Caller audit (audit's claim that no in-repo production caller exists): CONFIRMED.** A repo-wide search for `shm_read_ipv4|ramshield_shm_read` finds only the header itself and `scripts/shm_abi_stress.c:89` (a test/stress harness). The Rust side (`crates/ramshield-cgnat/src/shm.rs`) is a separate implementation; whether it has a tri-state read is **UNVERIFIED** (I did not read its read path in full within budget).

**ASSESSMENT.** Real contract defect, correctly scoped by the auditor as "confirmed source-level contract defect; actual production impact depends on callers". It is not a memory-safety issue and there is no demonstrated exploit in this repo. Severity: **P2 inside this repository** (the only caller is a stress harness; the Rust path is the production one), **P1 for the published C ABI**, which downstream consumers can misuse in exactly the way the audit describes. The fix is mechanical and already specified by the audit (tri-state result, prefix rejection before key calculation, boundary tests at 0/32/33/255, forced-odd-sequence test). It is already tracked (issue #30, draft PR #32) — a draft PR does not count as a fix on master. Owner decision needed: **yes**, on one point only — whether the ABI change to a tri-state return type is acceptable to consumers, i.e. a compatibility call, not a design call.

**ACTION.** Land the tri-state result + prefix rejection in `ramshield_ipv4_subnet_key`/`ramshield_shm_read` (`crates/ramshield-cgnat/include/ramshield_shm.h`) and the matching Rust read path in `crates/ramshield-cgnat/src/shm.rs`; close #30 only after the C ABI/stress gate and the Rust tests pass on one SHA.

---

### F2 — Native capture workers can stop without a durable health signal or restart

**STATUS: CONFIRMED.**

**EVIDENCE — spawn errors silently dropped**

`src/engine/native.rs:115-146`
```rust
let mut handles = Vec::with_capacity(workers);
for worker in 0..workers {
    …
    handles.push(
        std::thread::Builder::new()
            .name(format!("rs-native-{worker}"))
            .spawn(move || {
                if let Err(err) = run_linux(…) {
                    warn!(worker, error=%err, "native packet capture stopped");
                }
            }),
    );
}
for handle in handles.into_iter().flatten() {
    let _ = handle.join();
}
```
`handles` is `Vec<Result<JoinHandle<()>, io::Error>>`; `.flatten()` silently drops the `Err` arm, so a thread-creation failure is invisible. `handle.join()`'s result is discarded with `let _ =`.

**EVIDENCE — worker error = warn + exit, no restart, no degraded state**

`src/engine/native.rs:128-140` (quoted above): a `run_linux` error produces one `warn!` and the closure returns, ending the worker thread. The supervisor joins and returns; it never respawns and never publishes a health/degraded flag.

**EVIDENCE — `spawn()` can return success while every worker is already dead**

`src/engine/native.rs:96-111`: `spawn()` returns `std::io::Result<std::thread::JoinHandle<()>>` for the *supervisor* thread. On Linux it returns `Ok` as soon as the supervisor thread is created; worker creation happens later, inside the supervisor. `src/engine/boot.rs:619-632` only propagates the outer `Err`:
```rust
if cfg_snapshot.native_ingest.enabled
    && let Err(e) = crate::engine::native::spawn(…)
{
    return Err(std::io::Error::other(format!("native ingest start failed: {e}")));
}
```
So the process reaches ready state with capture already dead.

**EVIDENCE — the contrast the audit did not make, which raises the weight of the finding**

`src/engine/boot.rs:637-650` shows the *detection* workers fail closed:
```rust
let detection_started = detection.clone().spawn_workers(cfg_snapshot.engine.worker_threads);
if !detection_started {
    engine.pipeline_failed.store(true, Ordering::Release);
    engine.shutdown.store(true, Ordering::Release);
    …
    return Err(std::io::Error::other(
        "detection workers failed to start; refusing ready state",
    ));
```
Detection refuses to become ready when its workers die; native capture does not. That asymmetry is the real defect.

**EVIDENCE — no capture health surface.** A search of `src/dashboard/mod.rs` and `src/engine/mod.rs` for native/capture/status/health references returned nothing — there is no `/healthz` or module-status field reporting whether native capture is actually active. (`/healthz` and `/livez` are routed at `src/dashboard/mod.rs:54-55` on the audited SHA but report process liveness, not capture liveness.)

**ASSESSMENT.** Real defect, but an **observability/lifecycle gap, not a correctness or memory-safety bug** — the audit says as much. It requires the optional `native_ingest.enabled` feature to be on. Severity: **P2**, not P1: it does not corrupt state, does not fail open in a way that misrepresents enforcement *within* the native path, and is confined to an opt-in capture path; the impact is that an operator can believe capture is running when it is not. Owner decision: **yes, one** — restart the worker, fail the feature, or degrade the service. The observability part (count workers alive, expose it) is mechanical.

**ACTION.** In `src/engine/native.rs::spawn`, stop using `handles.into_iter().flatten()` (surface the `Err`), count live workers, and publish capture state; make `src/engine/boot.rs` treat "all workers exited" like the detection path (refuse ready state) or mark the module degraded.

---

### F3 — Native ingest drops events on a full channel without accounting at the producer

**STATUS: CONFIRMED as an observation; the audit's severity is overstated.**

**EVIDENCE**

`src/engine/native.rs:711`
```rust
    let _ = tx.try_send(ev);
```
The `Result` of `try_send` is discarded, so `TrySendError::Full` (and `Disconnected`) are never observed, let alone counted.

**No producer-side counter exists.** A repo-wide search for `events_shed` finds only the counter definition and readers, never an increment from the native path:
- `crates/ramshield-metrics/src/counters.rs:65` — `self.events_shed.fetch_add(n, Ordering::Relaxed);`
- `crates/ramshield-metrics/src/lib.rs:53`, `crates/ramshield-metrics/src/types.rs:64`, `crates/ramshield-metrics/src/prometheus.rs:83-84` — declaration/export only
- `src/engine/mod.rs:275`, `src/dashboard/mod.rs:455,457,478` — read/export only

There is no call site that increments it, so nothing anywhere accounts for a native `try_send` drop. The audit's statement that "the metrics crate documents rejected events generally, but the native producer's ignored send result prevents it from reporting this particular loss unless another independent counter exists" is exactly right: **no such independent counter exists**.

**ASSESSMENT.** The mechanism is real and it is a genuine observability gap: under a flood the operator sees no signal that packets were dropped before detection. But the behaviour itself — non-blocking `try_send` on a packet-ingest path, dropping on saturation rather than blocking the capture thread — is the *correct* overload policy for a real-time ingest path, as the audit itself concedes. Severity: **P2** (I would not call it P1/P2; it is P2, observability). Owner decision: **no** — purely mechanical: count `Full` separately from `Disconnected`, export the drop count and queue pressure, and document that `max_events_per_sec` is not a delivery guarantee.

**ACTION.** Replace `let _ = tx.try_send(ev);` in `src/engine/native.rs::process_packet` with a match that increments a new producer-side drop counter (and a `Full`-vs-`Disconnected` split) on the engine metrics.

---

### F4 — XDP LRU eviction is a real protection trade-off under unique-source floods

**STATUS: PARTIAL.** The *mechanism* the audit describes is confirmed. The audit's specific critique of the `MASTER_SCAN_NEGATIVE_RESULTS.md` wording is **partly misplaced** — it attributes the "safe direction" phrase to the LRU, but that phrase is about a different residual.

**EVIDENCE — LRU is real, and the map is bounded**

`crates/ramshield-xdp/ramshield-xdp-bpf/src/main.rs:24-30`
```rust
// LRU_HASH: kernel auto-evicts coldest entry when full — no E2BIG under botnet flood.
…
static BLOCKLIST: LruHashMap<[u64; 2], u64] = LruHashMap::with_max_entries(blocklist_cap_env(), 0);
static BLOCKLIST6: LruHashMap<[u64; 2], u64> = LruHashMap::with_max_entries(blocklist_cap_env(), 0);
```
`…/main.rs:90-99` — `blocklist_cap_env()` defaults to `102_400` entries. So the per-IP kernel map is a bounded LRU: a flood of unique source IPs evicts the coldest resident entry rather than failing the insert. An evicted IP's block is **absent**, i.e. that IP is no longer blocked — fail-open for that one IP, exactly as the audit says.

**EVIDENCE — reconciliation restores evicted expected keys**

`crates/ramshield-enforcement/src/xdp.rs:564-603` — `reconcile()` enumerates the map, deletes stale keys, and then "Insert ONLY the expected keys that are not already resident" (`xdp.rs:600-603`), and reports `evicted_count` (`xdp.rs:574,634,686`). So an LRU-evicted entry that is still in the expected set is re-inserted at the next reconcile. The exposure window for an evicted IP is therefore bounded by the reconcile interval, not unbounded.

**EVIDENCE — the document text the audit critiques**

`docs/development/MASTER_SCAN_NEGATIVE_RESULTS.md:276-282` (§7g, point 1):
> "**1. The per-IP kernel map is LRU — a flood cannot fill it.** … The kernel evicts the coldest entry rather than failing the insert, so a flood of unique source IPs degrades the oldest blocks instead of permanently breaking the map."

`…/MASTER_SCAN_NEGATIVE_RESULTS.md:306-308` (§7g, point 3):
> "**The residual is a bounded over-enforcement window of at most one reconcile interval (10 s)**, because the kernel-side entry has no TTL of its own. That is the safe direction for a mitigation system."

`…/MASTER_SCAN_NEGATIVE_RESULTS.md:325-328`:
> "**No change made.** The kernel and userspace block maps are bounded at every layer, the LRU absorbs per-IP floods, … and the one residual — a 10 s over-enforcement window on an expired prefix — is bounded and in the safe direction."

**Sentence-by-sentence judgement**

- "the LRU absorbs per-IP floods" and "a flood of unique source IPs degrades the oldest blocks instead of permanently breaking the map" — **defensible but imprecise / overclaiming.** "Degrades the oldest blocks" undersells the effect: LRU eviction *removes* the block, so the affected IP is unblocked, not merely degraded. And the "oldest" wording should be "coldest / least-recently-used" to match the kernel semantics quoted in the same document (`main.rs:24`). The document also never states the corresponding **under-enforcement** window (an evicted block stays absent until the next reconcile re-inserts it), even though it is careful to state the over-enforcement window for CIDRs.
- "That is the safe direction for a mitigation system" (`…/MASTER_SCAN_NEGATIVE_RESULTS.md:308`) — **defensible as written, and the audit's critique of it is misplaced.** The sentence is attached to the *CIDR* over-enforcement residual (an expired prefix that lingers for up to 10 s is over-blocking, which IS the safe direction for a mitigation system). It is not a claim about LRU eviction. The audit reads it as a general claim about eviction.
- "the LRU absorbs per-IP floods" + "No change made" (`:325-328`) — **the part that is genuinely overclaimed**: the conclusion sentence aggregates the LRU under "bounded at every layer" without stating the per-IP fail-open consequence that the audit identifies. *This* is the sentence that should be qualified, not the CIDR "safe direction" sentence.

**Also relevant: the finding is already-known-and-partly-parked.** §7g of the same document already establishes the LRU property, the reconcile repair, and the CIDR residual; `docs/development/MASTER_SCAN_NEGATIVE_RESULTS.md:398-404` (item L4, §7e) already records that `POST /api/config` applies to the *running* config and is reverted on restart. The audit's demand for eviction/saturation telemetry is new and is the only part not already covered.

**ASSESSMENT.** Accepted design trade-off, exactly as the auditor himself states ("This is an accepted design trade-off rather than a confirmed defect"). The audit's P1 label on a trade-off is inflation. The genuine actionable item is a **documentation-accuracy** one: `docs/development/MASTER_SCAN_NEGATIVE_RESULTS.md` §7g point 1 and its concluding sentence should (a) say "coldest/least-recently-used entry" and (b) state plainly that eviction is a per-IP fail-open bounded by the reconcile interval, with the corresponding under-enforcement window. Severity: **P3 (doc)**; the underlying trade-off needs no code change. Owner decision: **no** (mechanical doc edit), unless the operator wants eviction/repair telemetry, which is a real but optional enhancement.

**ACTION.** Amend `docs/development/MASTER_SCAN_NEGATIVE_RESULTS.md` §7g (points 1 and the "No change made" summary at `:325-328`) to state the per-IP fail-open consequence of LRU eviction and the reconcile-bounded recovery window; leave the CIDR "safe direction" sentence at `:308` unchanged.

---

### F5 — Dashboard is intentionally unauthenticated when no password hash is configured

**STATUS: CONFIRMED on the audited SHA `e5caac1`; ADDRESSED on the uncommitted working tree.** (Per the review brief, this is reported as "confirmed on the audited SHA; addressed in the uncommitted working tree", not as a refutation.)

**EVIDENCE — audited SHA `e5caac1`**

`src/dashboard/auth.rs` (at `e5caac1`), `enabled()`:
```rust
    pub fn enabled(&self) -> bool {
        self.password_hash
            .read()
            .map(|guard| guard.is_some())
            .unwrap_or(true) // Poisoned state must never disable authentication.
    }
```
and `require_auth` (at `e5caac1`):
```rust
    let auth = &state.auth;
    if !auth.enabled() {
        return next.run(req).await;
    }
```
With no `admin_password_hash`, `enabled()` is `false`, so the middleware short-circuits and **every** route passes through unauthenticated. The exemption list (`/healthz`, `/livez`, `/login`, `/metrics`) is not even reached.

`src/dashboard/mod.rs` (at `e5caac1`), routes:
```rust
.route("/api/config", get(api_get_config).post(api_set_config))
```
so the mutating `POST /api/config` is inside the open surface.

`crates/ramshield-config/src/validate.rs:367-381` (audited SHA) — the fail-closed guard applies only to a public bind:
```rust
        if is_public_bind(&self.dashboard.http_addr) && self.dashboard.admin_password_hash.is_none()
        { anyhow::bail!("dashboard.http_addr binds a public interface ({}) but admin_password_hash is unset …"); }
        if is_public_bind(&self.dashboard.http_addr) && !self.dashboard.tls_enabled { … }
```
`docs/SECURITY_MODEL.md:29-37` describes the loopback boundary honestly. So the audit's factual claims — "with no hash the auth middleware intentionally passes requests through", "POST /api/config lets any process that can reach the loopback listener change runtime configuration", "loopback is not a security boundary between mutually untrusted local users" — are all **correct**.

**EVIDENCE — uncommitted working tree already addresses it**

- `crates/ramshield-config/src/sections/dashboard.rs:32` `pub require_auth_for_config: bool,` with default `true` at `:102`
- `crates/ramshield-config/src/sections/ipc.rs:54` `pub require_auth_for_mutations: bool,` with default `true` at `:104`
- `src/dashboard/mod.rs:372-385` (working tree) — the config route is now gated:
```rust
    if state.engine.config.load().dashboard.require_auth_for_config
        && !auth::session_valid(&state.auth, &headers)
    { return (StatusCode::UNAUTHORIZED, Json(ConfigResponse { ok: false, … })); }
```
- `src/dashboard/auth.rs` (working tree) adds `pub fn session_valid(auth: &AuthState, headers: &HeaderMap) -> bool` and rewires `require_auth` to use it, so the cookie parsing cannot drift between the middleware and the route gate.
- `src/ipc/server/handlers.rs:82` — `&& engine.config.load().ipc.require_auth_for_mutations` for mutating IPC requests.
- Docs updated: `docs/OPERATIONS.md:92-100`, `docs/IPC.md:22-44`, `docs/CONFIGURATION.md:62-82`, `docs/SECURITY_MODEL.md`; the rationale is in `docs/development/OWNER_DECISION_PACKAGE.md:22` (Decision 1, option B) and `docs/development/POSTURE_CHANGE_REVIEW.md`.

**One consequence worth flagging.** With the working-tree defaults, a deployment with **no** `admin_password_hash` can never obtain a session (sessions are only registered after a successful password login, `src/dashboard/auth.rs` `register_session`/`validate`), so `POST /api/config` becomes permanently `401` in the default no-credentials configuration unless the operator sets `dashboard.require_auth_for_config = false` or configures a password. That is a defensible fail-closed posture and the escape hatch is documented, but it is a behaviour change for existing single-user dev setups and should be called out in the release notes.

**ASSESSMENT.** On the audited SHA this is a **real, deliberate security-posture finding** (P1): a loopback-bound, unauthenticated, state-mutating control plane. It is a *policy/design* item, not a coding bug — the code does what the security model says it does, and `docs/development/MASTER_SCAN_NEGATIVE_RESULTS.md:248-269` (§7b) shows the unauthenticated loopback default was a *tested* decision whose escalation path is closed by `validate()`. The audit is right that this is a trust-domain decision rather than a bug, and right that it did not establish a remote exposure. On the uncommitted working tree the finding is **addressed** by the new `require_auth_for_config` / `require_auth_for_mutations` gates, which close the local-mutator surface by default. Severity **on `e5caac1`: P1**; **on the working tree: mitigated** (residual: the escape hatch, and the fact that the *read* surface stays open, which is the intended design). Owner decision: **already taken** (Decision 1 = option B); what remains is verifying the exact route-authorization matrix in both modes, which the audit also asks for.

**ACTION.** On the working tree: land the `require_auth_for_config` / `require_auth_for_mutations` change (already implemented) and add the route-authorization matrix test for `POST /api/config` in authenticated and open modes; publish the "no password ⇒ config POST returns 401" behaviour change. On `e5caac1` alone the finding stands.

---

### F6 — BACKLOG.md is not a reliable master release ledger

**STATUS: CONFIRMED.**

**EVIDENCE** — `BACKLOG.md:5-13`
```
**Target branch:** `audit-branch`

**Baseline HEAD:** `70cd373f3c543ea796dd684197535956b49d6b0c`

**Baseline master:** `821eab72e93e584cd6835d1f95171d5d380daaec`

**Latest recorded CI:** [run 38081795011](…) — all CI jobs passed on exact HEAD `59d4bc6d68db132223e9817b61ac4c399b5e965e`, …
```
The file targets `audit-branch` and cites three SHAs (`70cd373`, `821eab7`, `59d4bc6`), none of which is the audited master `e5caac1`. The file's own status vocabulary (`BACKLOG.md`, "Status vocabulary") says *"Do not mark PASS from source inspection alone"* — which is the rule the file then breaks by carrying PASS rows pinned to a superseded SHA.

**ASSESSMENT.** Documentation / process defect, exactly as characterised: "useful as historical task context but unsafe as the sole answer to 'what is proven on current master?'". Severity **P2**. No code risk; the risk is a reader trusting a stale ledger. Owner decision: **no** — mechanical. Note the mitigation already in the file ("Operating rule: one bounded fix per pass; every fix is revalidated on the exact resulting SHA") shows the intent is right and only the header lags.

**ACTION.** Rewrite the `BACKLOG.md` header (lines 5-13) to point at `master` and at the current exact SHA, and split the historical `audit-branch` notes into a clearly-labelled appendix; re-stamp every current PASS row with its exact SHA and workflow URL.

---

### F7 — Performance and memory claims still require workload evidence

**STATUS: CONFIRMED for the configuration-value part; the PR #34 state is UNVERIFIED from here (remote-only artifact).**

**EVIDENCE** — `config.baseline.toml:15-17`
```
# Pin the engine's RAM budget explicitly (0 = schema default 512). 1024 =
# safe default for small VMs; raise for production traffic.
ram_limit_mb = 1024
```
`config.baseline.toml:42-43`
```
# Force-flush trigger on shared pre_aggs: 1M holds the full working set
# (bounded ~100 MB) — 8192 thrashed flushes under IP fanout.
pre_aggs_max_size = 1_000_000
```
Both are **configuration ceilings**, not measurements. `ram_limit_mb` is the engine's *budget* enforced at insert time (the store's insert path rejects with `RsError::CapacityExceeded` rather than growing — see `docs/development/MASTER_SCAN_NEGATIVE_RESULTS.md:339-345`), and `pre_aggs_max_size` is a flush trigger. Neither is an observed RSS figure, and neither can be one: the "~100 MB" in the comment is a sizing rationale for the aggregation working set, not a measured process RSS. The audit's characterisation is exact.

**What I could not determine:** the state of PR #34 and issue #35 is a **remote GitHub artifact** and I did not fetch it (read-only local repo, budget). The audit's own claim — that PR #34 is a **draft** and describes 50 MB as a *target*, not a demonstrated claim — is therefore **UNVERIFIED** in this review. It is consistent with the repo's own convention that unproven claims are labelled as targets, so I neither contradict nor endorse it.

**ASSESSMENT.** Correct as a **process/claims-hygiene** item, not a code defect. Severity **P2**. The audit's list of what must be recorded (exact SHA, profile/features, toolchain, config hash, machine, workload generator, packet sizes/rates, unique-source cardinality, duration, RSS vs. kernel/XDP maps vs. page cache, CPU/latency/drops, repeated runs with raw output) is a reasonable evidence template. The "keep sub-20ns microbenchmark language out of public claims" advice is a publishing rule, not a defect. Owner decision: **yes**, on the release-claims policy (what may be said publicly, and when), not on code.

**ACTION.** No code change. Add the evidence template to the release checklist and require it before any RSS/throughput figure is published; keep the `config.baseline.toml` comments explicitly labelled as ceilings (they already are, apart from the "~100 MB" sizing remark, which should read "≈100 MB working set by design, not measured RSS").

---

### F8 — No local reproduction in this audit pass

**STATUS: CONFIRMED (the audit is accurate about its own method).**

**EVIDENCE** — the audit's own text, `ramshield_master_audit_2026-10-21.md:11-13`
> "**Local reproduction:** not performed. A shallow clone was attempted but this runtime could not resolve `github.com`; therefore this is not a locally executed line-by-line audit."

and `:293-300`
> "I therefore did not run `cargo test`, `cargo clippy`, C SHM tests, live XDP tests, or crash-injection tests locally. The report distinguishes exact-SHA hosted evidence from source-level findings and does not claim local test execution."

Both statements are true of the document. The audit also states its severity labels are "an engineering judgment from the remote evidence reviewed, not a formal test result" (`:78-79`).

**ASSESSMENT.** Not a code defect; a **method caveat that caps the weight of every other finding**. It is the correct thing to have disclosed, and it was disclosed. Severity: **n/a (informational)**. The practical consequence, which the audit's own scorecard already reflects (Enforcement assurance 59/100, "lifecycle health need operational proof"), is that its P1/P2 labels are *unverified judgments*. My review reproduces that epistemic status: F1, F2, F3, F5, F6 are source-level confirmations and hold; the severities are mine, not test-derived. I could not run the C SHM ABI/stress gate, the Rust test suite, or any live XDP scenario, so I cannot certify that the proposed fixes (F1) do not break the existing ABI, and I make no claim about test-suite health.

**ACTION.** None against the code. Before any of the P1 items is closed, require the corresponding gate on one exact SHA (the audit's own exit conditions at `:335-337` are the right ones), and treat the audit's severity labels as hypotheses.

---

## 3. FINDINGS WORTH FIXING

Ordered by **my** severity.

1. **F5 — unauthenticated local control plane on the audited SHA (P1).** Already implemented on the uncommitted working tree (`dashboard.require_auth_for_config`, `ipc.require_auth_for_mutations`, both default `true`). Remaining work: land it, add the route-authorization matrix test, and publish the behaviour change (no password ⇒ `POST /api/config` returns 401).
2. **F1 — CGNAT SHM read cannot distinguish contention from a miss; invalid prefixes are routed into the table (P2 in-repo, P1 for the published C ABI).** Tri-state result + prefix rejection + boundary/forced-odd-sequence tests. Already tracked (#30, draft PR #32) — the draft does not count as fixed.
3. **F2 — native capture worker lifecycle has no failure signal (P2).** Surface spawn errors instead of `flatten()`, count live workers, publish capture health, and make `boot.rs` treat "all workers exited" like the detection path (refuse ready state / mark degraded).
4. **F3 — native `try_send` drops are invisible (P2).** Count `Full` separately from `Disconnected`, export the counters and queue pressure, document that `max_events_per_sec` is not a delivery guarantee.
5. **F6 — `BACKLOG.md` header is pinned to `audit-branch` and three superseded SHAs (P2, documentation).** Re-point at `master` and the current exact SHA; re-stamp every current PASS row.
6. **F7 — performance/RSS claims need workload evidence (P2, process).** Evidence template before any public figure; keep the baseline config's comments labelled as ceilings.
7. **F4 — `MASTER_SCAN_NEGATIVE_RESULTS.md` §7g wording (P3, documentation).** State the per-IP fail-open consequence of LRU eviction and the reconcile-bounded recovery window; say "coldest/least-recently-used", not "oldest".
8. **F8 — method caveat.** No action against the code; it caps the confidence of everything above.

---

## 4. FINDINGS WHERE I WOULD CHANGE THE SEVERITY

| # | Audit | Mine | Why |
|---|-------|------|-----|
| F4 | P1 | **P3 (doc)** | The audit itself calls it "an accepted design trade-off rather than a confirmed defect". P1 implies a defect; the only actionable residue is imprecise wording in `MASTER_SCAN_NEGATIVE_RESULTS.md` §7g, and the "safe direction" sentence the audit quotes (`:308`) is about the *CIDR over-enforcement* residual, where it is correct as written. |
| F2 | P1 | **P2** | Lifecycle/observability gap in an opt-in feature path (`native_ingest.enabled`); no state corruption, no fail-open misreport of enforcement *within* the native path, no demonstrated impact. It is more serious than P3 only because a silent capture death is invisible to operators. |
| F3 | P1/P2 | **P2** | `try_send`-and-drop is the correct overload policy for a packet-ingest path (the audit concedes this). The defect is purely the missing counter, and no independent counter exists. |
| F1 | P1 | **P2 in this repo / P1 for the C ABI** | The only in-repo caller is `scripts/shm_abi_stress.c`; the production path is the separate Rust implementation in `crates/ramshield-cgnat/src/shm.rs` (whose read path I did not verify). Severity depends on who consumes the published header. |
| F6 | P2 | **P2 (unchanged)** | — |
| F7 | P2 | **P2 (unchanged)** | — |
| F5 | P1 | **P1 on `e5caac1`; mitigated on the working tree** | Confirmed factually on the audited SHA; the working tree closes the mutating surface by default. |

**Severity I would *not* change:** F6, F7 (correctly P2 process/documentation items), and F8 (correctly presented as a method caveat, not a defect).

---

## 5. Caveats on this review

- Source-level only: no `cargo test`, `cargo clippy`, C SHM ABI/stress gate, live XDP test, or crash-injection test was run. I therefore certify what the code *says*, not how it *behaves*.
- Remote-only artifacts (PR #32, #34, #35, issues #30, and the CI run SHAs) could not be checked from the local clone; those specific claims are marked UNVERIFIED above.
- `crates/ramshield-cgnat/src/shm.rs`'s Rust read path was not read in full, so I do not claim the Rust implementation shares the C tri-state/prefix defects.
- The working tree contains an uncommitted change (14 files) that is **not** the subject of this review; it is cited only for F5, as instructed.

**REVIEWED IN 18 TOOL CALLS**
