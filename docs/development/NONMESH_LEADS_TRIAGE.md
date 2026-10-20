# NON-MESH Lead Triage (7 unverified scan leads)

- Repo: `/home/m/.cache/rs-master` (branch `fix/master-p0-p1-fixes`, HEAD `d41b286` == `origin/master`)
- Mode: read-only triage. No production code modified. Only this report was created.
- Method: targeted `grep -n` + region reads only. Every CONFIRMED/PARTIAL claim below is backed by quoted code.

## Summary

| Lead | Status | Severity | One-line finding |
|---|---|---|---|
| L1 replay-store bounds | REFUTED | — | Store is hard-bounded (global 1024 / per-key 256 / TTL 65 s), rejects on saturation, enforces TTL on lookup — it cannot grow unbounded and cannot re-accept a replayed frame. |
| L2 native XDP frame-size bounds | PARTIAL | P2 (unconfirmed) | The native path in this tree is the AF_PACKET TPACKET_V3 ingest (`src/engine/native.rs`); it *does* validate `blk_len`/`snaplen`/`mac` against `frame_size` and range-checks the slice. No separate "native XDP" frame parser exists that I could find; one sub-check (`tp_len` vs `snaplen`) was not fully verified. |
| L3 dashboard body limits | PARTIAL | P2 | No explicit `DefaultBodyLimit` layer, but axum 0.7's default 2 MiB extractor limit applies to the only two body-reading routes (`Json` on `/api/config`, `Form` on `/login`); dashboard defaults to loopback `127.0.0.1:9999`, and auth is optional (see L7). |
| L4 config-patch path | REFUTED | — | `POST /api/config` merges the patch, then rejects with 400 unless `cfg.validate()` passes *and* no redacted placeholder is present; it writes only to the engine's config handle, never to an arbitrary path. |
| L5 login handler | REFUTED | — | Argon2 (constant-time inside the KDF), per-IP lockout with a bounded failure table, semaphore-bounded CPU, and every error path returns "no session" — fails closed. Only residual gap: lockout is per-IP, so it is not a global brute-force throttle. |
| L6 `inc_blocks` counter | PARTIAL | P2 | `fetch_add` on `AtomicU64` is not saturating (wraps at 2^64 — practically unreachable) and there is exactly one increment site (no double counting), **but** it is incremented even when the XDP apply *failed*, so `blocks_total` counts attempts, not blocks actually applied. |
| L7 dashboard default/spawn | PARTIAL | P2 | Fails **open**: the dashboard is enabled by default, `admin_password_hash` defaults to `None`, and `require_auth` then passes every request through unauthenticated. Mitigation: the bind defaults to loopback, and `Config::validate()` refuses a public bind without a password hash. The "fail-closed" doc claim is therefore only true for public binds. |

---

## L1 — Replay-store bounds (`crates/ramshield-protocol/src/auth/replay_store.rs`)

**STATUS: REFUTED**

**EVIDENCE**

- The store carries three explicit bounds — `crates/ramshield-protocol/src/auth/replay_store.rs:24-32`:
  ```rust
  pub struct ReplayStore {
      cap: usize,
      per_key_cap: usize,
      ttl: Duration,
      ...
      inner: Mutex<StoreInner>,
  }
  ```
- Saturation **rejects** instead of evicting live markers — `replay_store.rs:102-113`:
  ```rust
  let own_key_count = g.order.iter().filter(|existing| existing.key_id_hash == key.key_id_hash).count();
  if g.map.len() >= self.cap || own_key_count >= self.per_key_cap {
      return Err("capacity");
  }
  g.map.insert(key.clone(), now);
  ```
  So a live nonce is never overwritten to make room for a new one; a replayed message cannot be re-accepted by eviction.
- TTL is enforced **on lookup**, not only on sweep — `replay_store.rs:95-100`:
  ```rust
  if g.map.get(&key).is_some_and(|prev| now.duration_since(*prev) < self.ttl) {
      return Err("replay");
  }
  ```
  plus lazy expiry sweep from the insertion-ordered front — `replay_store.rs:84-93`.
- Poisoning fails closed — `replay_store.rs:79`: `self.inner.lock().map_err(|_| "replay store poisoned")?` (test at `:177-189`).
- Production wiring: `src/ipc/server.rs:203-207`
  ```rust
  replay_store: Arc::new(ramshield_protocol::auth::ReplayStore::with_per_key_cap(
      1024, // global cap
      256,  // per-key cap
      Duration::from_secs(65),
  )),
  ```
  TTL 65 s ≥ 2 × `MAX_CLOCK_SKEW_MS` (10 s) + slack (comment `src/ipc/server.rs:194-202`).
- Replay protection is not optional at the call site — `crates/ramshield-protocol/src/auth.rs:91`: `replay.check_and_record(key_id, &expected)?;` (the store is a mandatory parameter of `verify`).

**IMPACT:** none. Memory is bounded at 1024 entries (≈33 KB of 32-byte digests), a replayed frame inside the skew window is rejected, and saturation produces `Err("capacity")` → the frame is rejected (fail-closed), not admitted.

**RECOMMENDATION:** No change required; close the lead as a false positive. (Optional micro-nit: `own_key_count` is an O(n) scan of `g.order` per insert — a per-key counter would be cheaper, but n ≤ 1024, so this is not a defect.)

---

## L2 — Native (non-eBPF) frame-size bounds

**STATUS: PARTIAL** (core claim not substantiated; the native path that does exist is bounds-checked)

**EVIDENCE**

- The only "native" (non-eBPF) packet path in this tree is the AF_PACKET TPACKET_V3 ingest: `src/engine/native.rs` (spawned from `src/engine/boot.rs:619-622` under `cfg.native_ingest.enabled`). `crates/ramshield-xdp/src/lib.rs` is the aya/eBPF side (map key layout helpers + aya map wrappers), not a native frame parser.
- Kernel-provided lengths are validated before use:
  - block length — `src/engine/native.rs:416-426`: `blk_len` is `usize::try_from(...hdr.blk_len)` and then `if !valid_tpacket_block_len(blk_len, block_size) { ... "rejecting invalid TPACKET_V3 block length" }`;
  - block offset — `src/engine/native.rs:382-390`: `block_idx.checked_mul(block_size)` and `checked_range_end(offset, block_size, ring_len)`, otherwise the ring is unmapped and the worker stops;
  - packet length — `src/engine/native.rs:462-479`:
    ```rust
    let snaplen = match usize::try_from(unsafe { (*hdr).tp_snaplen }) { ... };
    let mac = unsafe { (*hdr).tp_mac as usize };
    ...
    Some(value) if mac <= frame_size && snaplen <= frame_size => value,
    ```
    i.e. a `snaplen`/`mac` larger than the ring's `frame_size` (2048, `src/engine/native.rs:234` / `:335`) is rejected;
  - the slice handed to the parser is only formed after the range check — `src/engine/native.rs:495-497` (`packet_off + snaplen` must lie in the current block and the mapped ring).
  - ring geometry is overflow-checked — `src/engine/native.rs:240-243` (`ring_len > isize::MAX` → error) and `:347-349`.
- `snaplen`/`tp_len` are both present in the `tpacket3_hdr` struct (`src/engine/native.rs:84-85`); I confirmed `tp_snaplen` is bounded, but I did **not** fully verify that the copy length is `min(tp_snaplen, tp_len)` rather than `tp_snaplen` alone.

**IMPACT:** no confirmed defect. If the copy length were derived from `tp_len` (kernel-reported wire length) rather than the captured length, a truncated/offload frame could read past the captured bytes — but that path is not proven to exist. Severity if it were real: P2 (out-of-bounds read inside the ingest worker, ring is a fixed 8 MiB mmap).

**RECOMMENDATION:** One mechanical confirmation/fix in `src/engine/native.rs` (the packet-slice construction around `:495-497`): assert the copy length is `min(tp_snaplen, tp_len)` and that `tp_len <= frame_size`; add the same bound check for `tp_len` that already exists for `snaplen` at `:479`.

---

## L3 — Dashboard body limits (`src/dashboard/`)

**STATUS: PARTIAL**

**EVIDENCE**

- The dashboard binds an HTTP server with axum — `src/dashboard/mod.rs:33-92`; `Cargo.toml:76`: `axum = "0.7"`.
- Only two routes read a request body:
  - `src/dashboard/mod.rs:63` → `.route("/api/config", get(api_get_config).post(api_set_config))`, and the handler extracts `Json(patch): Json<ConfigPatch>` — `src/dashboard/mod.rs:286-290`;
  - `src/dashboard/auth.rs:476` → `Router::new().route("/login", get(login_page).post(login_submit))` with `Form(form): Form<LoginForm>` — `src/dashboard/auth.rs:373-378`.
- There is **no** explicit body-limit layer: a repo-wide grep for `DefaultBodyLimit` / `body_limit` in `src/` and `crates/` returns no hits. Axum 0.7's default `DefaultBodyLimit` (2 MiB) therefore applies to `Json` and `Form` extractors, which bounds allocation per request.
- The dashboard is loopback by default — `crates/ramshield-config/src/sections/dashboard.rs:74-76`:
  ```rust
  fn default_dashboard_http_addr() -> String { "127.0.0.1:9999".into() }
  ```
  and `DashboardConfig::default()` sets `enabled: true`, `admin_password_hash: None` (`sections/dashboard.rs:82-97`).
- Authentication is **optional** — see L7: with no `admin_password_hash` the `require_auth` middleware short-circuits (`src/dashboard/auth.rs:304-306`) and CORS is permissive (`src/dashboard/mod.rs:74-78`).
- Bind exposure is constrained by config validation — `crates/ramshield-config/src/validate.rs:368-378` (public bind + no password hash → `bail!`; public bind without `tls_enabled` → `bail!`).

**IMPACT:** unbounded allocation is not reachable (axum's 2 MiB default bounds it), so the lead's DoS claim is not substantiated. The real exposure is the *authentication* dimension (L7): an unauthenticated dashboard on loopback, or an authenticated one if the operator sets a hash. A 2 MiB body on an unauthenticated loopback socket is a low-severity local resource concern (P2).

**RECOMMENDATION:** One mechanical change in `src/dashboard/mod.rs:51-78`: add `.layer(DefaultBodyLimit::max(64 * 1024))` (or 16 KiB) to the router so the limit is explicit and enforced by code rather than by axum's default; `POST /api/config` and `POST /login` do not need more.

---

## L4 — Config-patch path

**STATUS: REFUTED**

**EVIDENCE**

- Handler: `api_set_config` — `src/dashboard/mod.rs:286-411`. Patch shape is a whole-section replacement of typed config structs — `src/dashboard/mod.rs:241-252`:
  ```rust
  pub struct ConfigPatch {
      pub engine: Option<crate::config::EngineConfig>,
      pub detection: Option<crate::config::DetectionConfig>,
      pub ipc: Option<crate::config::IpcConfig>,
      pub forecasting: Option<crate::config::ForecastingConfig>,
      pub dashboard: Option<crate::config::DashboardConfig>,
  }
  ```
- Sections are applied — `src/dashboard/mod.rs:369-388` — and then **validated** — `src/dashboard/mod.rs:389-398`:
  ```rust
  let invalid = cfg.validate().is_err() || contains_placeholder(&cfg);
  if invalid {
      return (StatusCode::BAD_REQUEST, Json(ConfigResponse { ok: false, ... }));
  }
  ```
  `cfg.validate()` is the same validator used at boot (`crates/ramshield-config/src/validate.rs:17`, `:308-378`), and it rejects an empty `http_addr`, `max_login_attempts == 0`, a public bind without `admin_password_hash`, and a non-loopback plaintext bind (`:356-378`, `:449-464`).
- Placeholder re-submission is rejected — `src/dashboard/mod.rs:358-368` (`contains_placeholder`).
- Cross-origin/CSRF gate runs before any mutation — `src/dashboard/mod.rs:305-349`; unauthenticated access is blocked by the `require_auth` middleware (`src/dashboard/auth.rs:298-355`).
- The write target is the engine's config handle only — `src/dashboard/mod.rs:399`: `state.engine.config.store(Arc::new(cfg.clone()));` plus a hot-swap of the password hash at `:401-403`. There is no path argument and no filesystem write in this handler, so "write outside the config file" is not possible.
- The password hash is re-validated at the Argon2 layer before use: an invalid PHC string simply fails every verify (`src/dashboard/auth.rs:247`, `PasswordHash::new(&hash).ok()?` → `None` → no session).

**IMPACT:** none confirmed. An unvalidated write is not reachable: the patch is typed, validated, CSRF-checked, and auth-gated.

**UNVERIFIED sub-point:** whether `engine.config.store(..)` also persists the new config to disk was not inspected (the call site shows an in-memory config handle). If it is memory-only, dashboard config edits are lost on restart — an operational surprise, not a security hole. I did not have tool-call budget to inspect `ConfigStore::store`.

**RECOMMENDATION:** No security change required. If disk persistence is desired for dashboard edits, add it explicitly (and write through a temp-file + rename into the config path) — owner decision.

---

## L5 — Login handler (`src/dashboard/auth.rs`)

**STATUS: REFUTED**

**EVIDENCE**

- Password comparison is Argon2 (constant-time inside the KDF, no byte-wise compare of secret material) — `src/dashboard/auth.rs:243-264`:
  ```rust
  let hash = self.password_hash.read().ok()?.as_ref()?.clone();
  let parsed = argon2::PasswordHash::new(&hash).ok()?;
  if password.len() > self.max_password_length { return None; }
  let ok = { use argon2::password_hash::PasswordVerifier;
             argon2::Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok() };
  if !ok { return None; }
  ```
  Work is capped on garbage input by `max_password_length` (default 1024, `sections/dashboard.rs:66-68`).
- Rate-limit / lockout: per-IP, `LOCKOUT_WINDOW = 15 min` (`src/dashboard/auth.rs:81`), checked before any hashing — `src/dashboard/auth.rs:402-409`:
  ```rust
  if auth.is_locked(ip) { auth.metrics.inc_auth_lockout(); return (StatusCode::TOO_MANY_REQUESTS, "locked").into_response(); }
  ```
  `is_locked` (`:187-199`) counts failures per IP; `note_failure` (`:201-237`) increments with `saturating_add` and the failure table is hard-bounded at `MAX_TRACKED_FAILURE_IPS = 10_000` (`:206`, `:223-229`) with eviction instead of growth.
- The client IP key is only taken from `X-Forwarded-For` when the TCP peer is a configured trusted proxy — `src/dashboard/auth.rs:387-401` (CWE-307).
- CPU is bounded: Argon2 runs on the blocking pool under a semaphore — `src/dashboard/auth.rs:413-431` (`argon2_semaphore`, `argon2_parallelism`, default 4, `sections/dashboard.rs:70-72`).
- Fails closed on every error path: poisoned lock → `read().ok()?` → `None` (`:246`); invalid PHC → `None` (`:247`); task join error → `.unwrap_or(None)` (`:430-431`); `None` → no session, failure recorded, 401 returned (`:457-470`). Sessions are only registered on the success branch (`:437`), are 32 random bytes (`:261-263`), and are TTL-checked exactly on every use (`:273-294`).

**IMPACT:** none confirmed. Residual: lockout is per-IP, so a distributed spray across many source IPs is not throttled by this mechanism (the only global control is the Argon2 semaphore). P2, and arguably an accepted policy position.

**RECOMMENDATION:** No code change required. If a global brute-force ceiling is wanted, that is a policy decision: add a global (non-per-IP) failure counter in `AuthState::note_failure` (`src/dashboard/auth.rs:201`).

---

## L6 — `inc_blocks` counter (`crates/ramshield-metrics/`)

**STATUS: PARTIAL**

**EVIDENCE**

- The counter is a plain `fetch_add` on an `AtomicU64` — `crates/ramshield-metrics/src/counters.rs:8-10`:
  ```rust
  pub fn inc_blocks(&self) {
      self.blocks_total.fetch_add(1, Ordering::Relaxed);
  }
  ```
  backed by `pub blocks_total: Arc<AtomicU64>` (`crates/ramshield-metrics/src/lib.rs:29`, initialised at `:192`). `fetch_add` **wraps** on overflow — it is not `fetch_update`/`saturating`. At 1 M blocks/s, 2^64 blocks is ≈584 000 years away, so the wrap is not practically reachable; no other counter in the file uses a saturating pattern either.
- No double counting: there is exactly **one** increment site in the whole tree — `crates/ramshield-enforcement/src/service/enforce.rs:353` (`grep -rn 'inc_blocks\('` → only `counters.rs:8` and `enforce.rs:353`). `inc_blocks_expired` (`counters.rs:143`) is a different counter.
- **The real finding:** the increment sits *after* the XDP-apply result is computed and is executed unconditionally — `crates/ramshield-enforcement/src/service/enforce.rs:322-353`:
  ```rust
  let xdp_applied = match cmd.cidr { ... }
      .map(|()| true)
      .unwrap_or_else(|e| {
          self.metrics.mark_xdp_projection_stale();
          self.metrics.inc_xdp_apply_failures();
          ...
          false
      });
  self.metrics.inc_blocks();
  ```
  So a block whose kernel projection **failed** (e.g. CIDR LPM trie full) still increments `blocks_total`. The same code path logs the loud error, but the scraped counter `ramshield_blocks_total` (`crates/ramshield-metrics/src/prometheus.rs:35-36`) over-reports blocks actually in force.

**IMPACT:** P2 — metric correctness, not memory safety or an attack surface. `blocks_total` over-counts during kernel-projection divergence (exactly the incident the code above is trying to surface), so an operator reading `/metrics` or the dashboard's `blocks_total` (`src/dashboard/mod.rs:454`) sees more blocks applied than the kernel holds. The overflow half of the lead is not practically exploitable.

**RECOMMENDATION:** One mechanical change in `apply_block_projection` (`crates/ramshield-enforcement/src/service/enforce.rs:353`): only call `self.metrics.inc_blocks()` when `xdp_applied` is true, and add a separate `inc_block_apply_failures`-style counter (already present as `inc_xdp_apply_failures`) for the failure side.

---

## L7 — Dashboard default / spawn behaviour

**STATUS: PARTIAL** — the docs' "fail-closed" claim holds only for public binds; the default deployment is *open* (unauthenticated) on loopback.

**EVIDENCE**

- The dashboard is enabled by default and has no password by default — `crates/ramshield-config/src/sections/dashboard.rs:82-97`:
  ```rust
  Self { enabled: true, http_addr: "127.0.0.1:9999".into(), admin_password_hash: None, ... }
  ```
- It is spawned whenever enabled, with no password check — `src/main.rs:171-196`:
  ```rust
  if dashboard_config.enabled {
      ... dashboard::serve(eng_clone, &dashboard_config.http_addr, &config_clone).await ...
  }
  ```
  `dashboard::serve` (`src/dashboard/mod.rs:33-92`) never inspects `admin_password_hash` for presence; it just constructs `AuthState`.
- With no hash, authentication is **disabled** and every request passes — `src/dashboard/auth.rs:160-165`:
  ```rust
  pub fn enabled(&self) -> bool {
      self.password_hash.read().map(|guard| guard.is_some()).unwrap_or(true) // Poisoned state must never disable authentication.
  }
  ```
  and `src/dashboard/auth.rs:304-306`:
  ```rust
  if !auth.enabled() { return next.run(req).await; }
  ```
  (note the poisoned-lock case correctly fails *closed*, per the comment).
- CORS becomes permissive in that state — `src/dashboard/mod.rs:74-78`:
  ```rust
  .layer(if app_state.auth.enabled() { CorsLayer::new() } else { CorsLayer::permissive() });
  ```
- The mitigating constraint: a public bind without a password hash is rejected by config validation — `crates/ramshield-config/src/validate.rs:368-373`:
  ```rust
  if is_public_bind(&self.dashboard.http_addr) && self.dashboard.admin_password_hash.is_none() {
      anyhow::bail!("dashboard.http_addr binds a public interface ({}) but admin_password_hash is unset — set the hash or bind 127.0.0.1", ...)
  }
  ```
  plus `:375-378` (public bind requires `tls_enabled`) and `:449-464`. Validation runs on the file path and on the no-config path (`src/main.rs:100`).
- The `api_set_config` path cannot be used to sneak into an open state either: it re-validates the merged config (`src/dashboard/mod.rs:389`), so setting a public bind without a hash is rejected with 400.

**IMPACT:** P2. Failure mode: an operator who enables the dashboard (the default) with no `admin_password_hash` gets a fully unauthenticated API — including `POST /api/config` (which can change engine/IPC/detection config, including setting `ipc.auth_keys` and the dashboard's own `http_addr`) — reachable from anything that can reach `127.0.0.1:9999` (any local process/user on the host, plus any SSRF in a co-located service). The blast radius is contained to the local host because the default bind is loopback and public binds without a hash are rejected at boot. If the dashboard is exposed through a reverse proxy to a non-loopback interface while the proxy supplies the loopback bind address, the config-level guard is bypassed by the deployment topology — that is the realistic P1 scenario and it is a **policy/owner decision**, not a code bug.

**RECOMMENDATION:** Owner decision + one mechanical warning: decide the policy for "dashboard enabled, `admin_password_hash` unset" (recommended: keep serving on loopback but log a prominent `warn!` at startup in `src/main.rs:176` / `dashboard::serve` at `src/dashboard/mod.rs:34`, and require an explicit `dashboard.allow_unauthenticated = true` opt-in to silence it); if the policy is "fail closed", refuse to start the dashboard thread in `src/main.rs:176` when the hash is absent.

---

## RECOMMENDED NEXT FIXES (ordered by severity)

1. **L7 — dashboard fails open with no admin password hash (P2, policy).** Decide the policy; mechanically, add a loud startup warning in `src/main.rs:176` and/or an explicit opt-in flag, and document the reverse-proxy-bypass scenario.
2. **L6 — `blocks_total` counts failed XDP projections as blocks (P2, mechanical).** In `crates/ramshield-enforcement/src/service/enforce.rs:353`, increment only when `xdp_applied == true`.
3. **L3 — no explicit body limit on the dashboard router (P2, mechanical).** Add `.layer(DefaultBodyLimit::max(64 * 1024))` to the router in `src/dashboard/mod.rs:51-78`.
4. **L2 — confirm the native ingest copy length (P2, mechanical/unverified).** In `src/engine/native.rs` (packet-slice construction near `:495-497`), bound the copy by `min(tp_snaplen, tp_len)` and add the `tp_len <= frame_size` check that already exists for `snaplen` at `:479`.
5. **L4 — confirm dashboard config-edit persistence (P2, mechanical/doc).** Check `engine.config.store(..)` semantics (`src/dashboard/mod.rs:399`); if memory-only, document that edits do not survive a restart.
6. **L5 — per-IP-only lockout (P2, policy).** Optional global failure ceiling in `AuthState::note_failure` (`src/dashboard/auth.rs:201`).
7. **L1, L4 (security claims) — no action.** Close as false positives; they were hardened in earlier work and the code proves it.

**TRIAGED IN 15 TOOL CALLS** (14 investigation calls + 1 report write; the write is the last call, all evidence above was gathered before it).
