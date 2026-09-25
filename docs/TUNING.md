# RamShield Tuning

**Audience:** Operators adjusting production behavior. **Last verified:** 2026-09-25 · **Gate:** `cargo check --all-targets --all-features`

> Every parameter below is a real config key from `config.toml` (derived from the canonical
> template `config.baseline.toml`). The `⚙️` emoji flags keys commonly tuned in production.
> Always restart after a config change; env overrides `RAMSHIELD_*__FIRE userspace. But the error says missing required field 'path'. I need to set the correct path parameter. I need to just go ahead and do this.

Let me restart with a simpler approach. The user wants me to proceed with writing documentation. Let me just do it step by step by step.

First, let me check what docs already exist and what structure I should follow.
<tool_call>
<function=terminal>
<parameter=command>
cd /home/m/vehicle_of_rationalismark config-checks and env overrides.

## 1. Engine (resource budget)

| Key | Default | Trade-off / When to change |
|---|---|---|
| `engine.worker_threads` | `0 (= auto / CPU count)` | Set to `1` if you have >8 cores and want a dedicated batch thread instead of auto-detect. Rarely needed. |
| `engine.ram_limit_mb` | `512` | **⚙️** Lower to fit in a constrained VM (min 64 MB). Raise if store insertion returns `CapacityExceeded`. Do not exceed host RAM. |
| `engine.shard_count` | `256` | Must be a power of two. Only increase if profiling shows shard-lock contention; the cost grows ~O(N) per shard. |

## 2. Detection (EWMA / subnet / bloom / pulse)

| Key | Default | Trade-off / When to change |
|---|---|---|
| `detection.rps_threshold` | `500` | **⚙️** Raise if legitimate traffic exceeds threshold; lower if false blocks are frequent. |
| `detection.rate_window_secs` | `10` | Sliding window for count decay. Larger window = smoother EWMA; smaller = more responsive. |
| `detection.subnet_batch_threshold` | `50` | **⚙️** Minimum distinct source IPs per /24 before subnet block considered. Raise if you want fewer subnet blocks; lower if you want more. |
| `detection.subnet_batch_min_events` | `100` | **⚙️** /24 event volume gate (must also pass threshold). Higher = fewer subnet blocks. |
| `detection.batch_block_enabled` | `true` | Disable if you only want IP-scale blocking. |
| `detection.block_ttl_secs` | `300` | Auto-block TTL. Affects how long an IP stays blocked after threshold crossing. Lower = faster unblock; raise for long-lived attacks. |
| `detection.bloom_bits` | `8_000_000` | **⚙️** Bloom capacity (bits) — larger = lower false-positive estimate. A useful rule of thumb: `~20 × promoted IPs per 8 s epoch`. |
| `detection.batch_max_events` | `4096` | Max events per flush batch. Raise if you see backpressure (`events_shed_total`) under extreme load. |
| `detection.batch_window_ms` | `50` | Max wait before flushing a partial batch. Larger = more amortization; smaller = lower latency. |
| `detection.promote_min_events` | `8` | **⚙️** Hits in one window before a full `IpRecord` is stored. Lower = more IPs tracked; raise to reduce RAM. |
| `detection.subnet_window_threshold` | `500` | /24 event count that lowers promotion threshold for that subnet. Raise if /24-promoted IPs are too noisy. |
| `detection.subnet_burst_ttl_secs` | `120` | Subnet-burst block TTL. **Shorter than** `block_ttl_secs` — prevents whole shared-egress /24s staying locked out an hour. |
| `detection.pulse_window_secs` | `5` | Pulse-wave detection window. |
| `detection.pulse_threshold_samples` | `2` | Number of over-threshold samples to trigger a pulse-wave block. Raise if FPR observed. |
| `detection.emergency_burst_threshold` | `500` | **⚙️** One-IP fast-path block if this many events seen in the unflushed window. Raise if you want the fast path to fire less often. Set `0` to disable. |
| `detection.pre_aggs_max_size` | `1_000_000` | Max events in pre-aggregation buffer before flush. Raise if batches are forced early. |

## 3. IPC (auth + networking)

| Key | Default | Trade-off / When to change |
|---|---|---|
| `ipc.tcp_addr` | `127.0.0.1:7890` | **⚠️** Change to `0.0.0.0:7890` only if you front with TLS + auth keys; otherwise the daemon will refuse to start (fail-closed public-bind guard). |
| `ipc.max_connections` | `256` | Concurrent IPC connections. Raise if you get `rejected_connections` under load. |
| `ipc.max_line_length` | `33554432` (32 MB) | Max single JSON line. Do not reduce unless you want to reject giant payloads. |
| `ipc.auth_keys` | `[]` (empty) | **⚠️** Must be `key_id:hex_key` pairs when `ipc.require_auth = true`. Put real keys in a gitignored overlay (e.g. `config.prod.toml`) or via `RAMSHIELD_IPC__AUTH_KEYS` env. Empty + loopback = open server (dev default). |
| `ipc.require_auth` | `false` | **⚠️** If `true` and `auth_keys` is empty → refuses to start. Use in CI/staging to enforce auth coverage. |
| `ipc.read_timeout_ms` / `write_timeout_ms` | `5000` | Per-connection idle timeout. Raise if clients disconnect prematurely. |
| `ipc.connection_idle_timeout_ms` | `30000` | Accepted connection idle timeout. |

## 4. Dashboard (HTTP + auth)

| Key | Default | Trade-off / When to change |
|---|---|---|
| `dashboard.enabled` | `true` | Disable only if you have another UI or never need a web UI. |
| `dashboard.http_addr` | `127.0.0.1:9999` | **⚠️** Bind to `0.0.0.0` or a public IP only if you have TLS in front; otherwise `validate()` will reject. |
| `dashboard.block_log_size` | `1000` | Ring buffer entries served by `/api/history/blocks`. Raise if history scrolls out too fast during floods. |
| `dashboard.admin_password_hash` | `None` | **⚠️** Argon2 PHC hash of the admin password. Generate: `echo -n 'pw' | argon2 "..." -id -e`. Without it, `/login` is open. Required when dashboard binds non-loopback. |
| `dashboard.session_ttl_secs` | `28800` | 8h session lifetime. Lower if you want shorter sessions. |
| `dashboard.max_login_attempts` | `50` | Lockout after N failures. |
| `dashboard.max_password_length` | `1024` | Max password length — Argon2 work is bounded, but long passwords inflate cookie size. |
| `dashboard.trusted_proxies` | `[]` | IPs allowed to forward `X-Forwarded-For` for client-IP attribution. CWE-307 fix. |
| `dashboard.tls_enabled` | `false` | **⚠️** Only set `true` when behind an HTTPS reverse proxy. Without it, browsers silently drop `Secure` cookies on non-loopback HTTP → infinite login loops. |
| `dashboard.cookie_secure` | `None` | Auto-derived: `Some(true)` for loopback bind, `Some(false)` for non-loopback, `None` for default. Override only if you have a bespoke TLS setup. |

## 5. WAL (crash durability)

| Key | Default | Trade-off / When to change |
|---|---|---|
| `wal.enabled` | `false` | **⚙️** Enable to persist block state across restarts. Disabled = blocks lost on `SIGKILL`. |
| `wal.dir` | `"/var/lib/ramshield/wal"` | WAL directory. Must be writable and have space for `retention_max_bytes`. |
| `wal.durability` | `GroupCommit` | `GroupCommit` = fsync amortized across concurrent commits; `Flush` = fdatasync on every block. Use `Flush` for strictest durability. |
| `wal.compress` | `true` | LZ4 segment compression. Disable only if CPU is saturated. |
| `wal.seg_max_bytes` | `67_108_864` (64 MiB) | Max segment size. |
| `wal.retention_max_bytes` | `536_870_912` (512 MiB) | Oldest segments deleted first. Set `0` for unlimited (watch disk). |

## 6. XDP (kernel dataplane)

| Key | Default | Trade-off / When to change |
|---|---|---|
| `xdp.enabled` | `false` | **⚙️** **Fail-closed default.** Enable only if you have the kernel + NIC + capabilities. Without XDP, userspace blocking and all APIs still work — only the kernel drop path is absent. |
| `xdp.interface` | `eth0` | Network interface. Change to `lo` for loopback/XDP-generic testing (no NIC needed). |
| `xdp.mode` | `"skb"` | `"skb"` works on generic drivers; `"drv"` requires native NIC mode + `CAP_BPF`. |
| `xdp.build_mode` | `auto` | `auto` tries aya-build first, then clang C, then a stub ELF guaranteeing `cargo check` never fails. Override to `rust`, `clang`, or `stub` if you have a preferred path. |

## 7. Forecasting (Holt-Winters + entropy)

| Key | Default | Trade-off / When to change |
|---|---|---|
| `forecasting.enabled` | `true` | Disable only if you never use anomaly-driven blocking. |
| `forecasting.ewma_alpha` | `0.3` | EWMA level smoothing. `0 < α ≤ 1`. Lower = smoother; raise if tracking fast bursts. |
| `forecasting.hw_beta` / `hw_gamma` | `0.1` / `0.1` | Trend + seasonal smoothing. |
| `forecasting.seasonality_period` | `3600` (1 hour) | Seasonality cycle in seconds. Raise if your attack pattern has a longer cycle (e.g., daily). |
| `forecasting.anomaly_zscore` | `2.5` | z-score threshold for `ForecastAnomaly`. Lower = more sensitive; raise if FPR observed. |
| `forecasting.min_entropy` | `2.0` | Minimum Shannon entropy (bits) for `EntropyAnomaly`. Lower = more sensitive to uniformity. |