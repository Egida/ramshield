# RamShield Features

**Audience:** Deployers evaluating RamShield. **Last verified:** 2026-09-25 · **Gate:** `cargo check --all-targets --all-features`

> RamShield is an in-memory DDoS detection + mitigation daemon. It ingests connection
> telemetry, detects floods and swarms, and pushes block decisions to the Linux
> kernel via eBPF/XDP. Every feature listed here is real code — named, referenced,
> and verified against the source it documents.

## 1. Ingest

### 1.1 Authenticated JSON/TCP IPC
- One JSON object per line over TCP. Requests are `snake_case-typed` (see `docs/IPC.md`).
- **HMAC-SHA256 per-frame authentication** (`crates/ramshield-protocol/src/auth.rs`): every frame carries `{"auth":{"key_id","ts_ms","sig"}}` when `[ipc] auth_keys` is non-empty.
  - Signature covers `<ts_ms>.<key_id>.<payload>`; `key_id` is bound into the MAC so two keys with identical bytes sign differently.
  - Constant-time compare; **replay protection is mandatory in production** via `verify_authenticated` — requires a `&ReplayStore`, never `None` (P3).
  - Clock-skew window: **±10 s** (`MAX_CLOCK_SKEW_MS = 10_000`).
- **Role-based authorization** (P2): `key_roles` config assigns a `KeyRole` per `key_id` (`Telemetry` < `ReadOnly` < `Operator` < `Admin`). IPC enforces before dispatch; insufficient role → `403`, unauthenticated → `401`.
- **Authenticated identity** (P1): the verified `key_id` becomes the `actor` on every enforcement command — no hard-coded `"admin"`.
- **Transport bind safety** (P4): non-loopback `ipc.tcp_addr` without `ipc.behind_tls_proxy = true` fails at startup. HMAC authenticates; it does not encrypt.
- **`deny_unknown_fields`** — unknown JSON fields reject the frame (typos fail loudly, no silent misparse).

### 1.2 Single + batch report
- `report_connection` (one event) and `report_connections` (N events in one frame). Batch matches the detection batching path on the wire, cutting syscalls 10–100×.

## 2. Detection

All detection is **bounded-memory** — state lives in a 256-shard ahash map with a hard RAM ceiling; cold IPs never touch the store.

### 2.1 EWMA rate tracking
- Per-IP smoothed requests/sec via EWMA (`ALPHA = 0.3`). Smoothes burst noise while reacting to sustained abuse.
- Tripwire: `detection.rps_threshold` (default 1000). Crossing → automatic block.

### 2.2 Holt-Winters forecasting
- Seasonality-aware trend + level + seasonal smoothing (`hw_beta = 0.1`, `hw_gamma = 0.1`, `seasonality_period`).
- z-score anomaly: `|z| > anomaly_zscore` (default 2.5) → `ForecastAnomaly` block, TTL short by default.

### 2.3 Entropy (Swarm) detection
- Shannon entropy over the /24 subnet distribution. **Low entropy = botnet-like uniformity** → `EntropyAnomaly`.
- Floor: `min_entropy` (default 2.0 bits).

### 2.4 Subnet / dual-gate swarm blocking
- **Dual gate**: a /24 is blocked only when **both** `unique_ips >= subnet_batch_threshold` (default 50) **and** `events >= subnet_batch_min_events` (default 100). Keyed on *distinct source IPs*, not raw events — one abuser at 500 events is a lone offender; 50 IPs × 12 events is a swarm.
- **`subnet_burst_ttl_secs`** (default 120) is intentionally shorter than per-IP TTL so whole shared-egress /24s don't stay locked out an hour.

### 2.5 Pulse-wave (persistent low-rate) detection
- Catches ramp-and-burst profiles that stay under the static tripwire: bursts spaced just below threshold. Window `pulse_window_secs` (default 6), escalation after `pulse_threshold_samples` (default 2) over-threshold samples.

### 2.6 Emergency burst fast path
- `emergency_burst_threshold` (default 500): one IP emitting this many events in a single unflushed window triggers an **in-flight block immediately** instead of waiting for the periodic flush (saves 50–1000 ms of uninhibited traffic).

### 2.7 Promotion filter (cold-IP skipping)
- An IP gets a full `IpRecord` only if any of:
  - `agg.count >= promote_min_events` (default 8),
  - its /24 is hot in the window (`subnet_window_threshold`, default 500 events),
  - it hits the **bloom filter** (advisory revisit cache over promoted IPs).
- Memory and CPU are proportional to *active threats*, not total unique sources.

## 3. Enforcement

### 3.1 Single-writer enforcement service
`EnforcementService` is the **sole writer** of block state. One command queue, one actor. No concurrent writers → no race on the blocklist.

- **Idempotency**: `decision_id` (UUID) — a replayed decision returns the cached original result, never a fabricated success.
- **Deduplication**: in-memory `blocked_ips` set prevents redundant XDP map operations.
- **WAL-first ordering**: append to WAL → mutate store → update local/XDP indexes. A failed storage mutation must not leave a phantom block; a failed WAL append aborts before any state change.
- **WAL durability**: `GroupCommit` (fsync amortized) or `Flush` (per-write). State survives `SIGKILL` and replays at boot.

### 3.2 CIDR blocking (IPv4 + IPv6)
- Block/unblock whole prefixes (`block_cidr` / `unblock_cidr`). CIDRs live in a separate LPM-trie map from per-IP entries; a member host needs no `IpRecord` — the shared set is the block.

### 3.3 eBPF/XDP dataplane
- Block prefixes pushed into kernel maps; the XDP program drops packets on the wire **before the socket layer**.
- IPv4 + IPv6 drop paths, per-CPU counters, RingBuf drop-notification feedback (attribution gap metric reports kernel/userspace drift).
- **XDP is an enforcement boundary, not the detector.** Without XDP (or in a container lacking `CAP_BPF`), userspace blocking and all APIs still work — only the kernel drop path is absent.

## 4. Observability

### 4.1 Prometheus `/metrics`
Exported `ramshield_*` metrics (exact names, from source):

| Metric | Type | Meaning |
|---|---|---|
| `ramshield_requests_total` | counter | IPC requests received |
| `ramshield_blocks_total` | counter | Total blocks issued |
| `ramshield_events_ingested_total` | counter | Events ingested |
| `ramshield_frames_rejected_total` | counter | Frames rejected at parse/auth |
| `ramshield_events_rejected_total` | counter | All rejections (401s + refusals + drops) |
| `ramshield_ipc_event_drops_total` | counter | Channel-full clean subset |
| `ramshield_ipc_auth_rejections_total` | counter | 401 frames |
| `ramshield_ipc_rejected_connections_total` | counter | Accept-semaphore refusals |
| `ramshield_events_shed_total` | counter | Low-signal events shed at HWM |
| `ramshield_ingest_channel_depth` | gauge | Buffered events in bounded queue |
| `ramshield_active_cidr_blocks` | gauge | CIDR prefixes in kernel LPM trie |
| `ramshield_batches_total` | counter | Detection batches processed |
| `ramshield_promotions_total` | counter | IPs promoted to tracking |
| `ramshield_cold_skipped_total` | counter | Cold IPs skipped |
| `ramshield_blocks_detection` / `_subnet` / `_forecast` | counter | Block source |
| `ramshield_enforcement_dropped_total` | counter | Block commands dropped on full channel — attacker unblocked in kernel |
| `ramshield_hw_rps` / `hw_zscore` / `hw_forecast` | gauge | Holt-Winters state |
| `ramshield_entropy` | gauge | Current IP entropy (bits) |
| `ramshield_bloom_*` (`bits`, `inserts_epoch`, `clears_total`, `saturation_clears_total`, `fp_ppm`) | gauge/counter | Bloom advisory-cache health |
| `ramshield_xdp_v4_drops` / `xdp_v6_drops` / `xdp_attribution_gaps` | counter | Kernel drop accounting |
| `ramshield_cgnat_*` | counter | CGNAT tier verdicts (allow/challenge/powdrop/block) |
| `ramshield_shm_*` | counter | Shared-memory rule-table publishes/lookups/hits |
| `ramshield_hll_insert_total` / `cms_increment_total` | counter | Cardinality / count-min sketches |
| `ramshield_mesh_record_*` / `purge_ticks` / `hlc_ticks` | counter | Mesh CRDT |

### 4.2 Dashboard
- Browser UI at `[dashboard].http_addr` (default `127.0.0.1:9999`).
- **Argon2id-protected login**; session-cookie middleware; CSRF-checked config POST.
- Server-Sent Events stream (`/api/stream`), Prometheus `/metrics`, healthz.

## 5. Security controls

| Control | Where |
|---|---|
| IPC HMAC-SHA256 frame auth | `crates/ramshield-protocol` |
| IPC `deny_unknown_fields` | `crates/ramshield-protocol/src/message.rs` |
| Dashboard Argon2id password hash | `[dashboard].admin_password_hash` |
| Fail-closed public binds | `config validate()` — public bind without auth refuses startup |
| Trusted-proxy IP attribution | `[dashboard].trusted_proxies` (CWE-307 fix) |
| Secure-cookie loopback auto-derive | `[dashboard].cookie_secure` |

## 6. What it is not

RamShield is **not** a general DB, not a full firewall, and not a packet-capture tool. It is a RAM-first detection engine plus an XDP enforcement shim. Detection decisions originate in **userspace**; enforcement pushes them into the kernel.