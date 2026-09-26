1|# Configuration
2|
3|Full reference for every configuration field. The canonical source of truth is
4|[`config.baseline.toml`](../config.baseline.toml) and
5|[`crates/ramshield-config/src/lib.rs`](../crates/ramshield-config/src/lib.rs).
6|
7|Overrides: environment variables (`RAMSHIELD_SECTION__KEY`) take precedence over TOML values.
8|
9|## `[engine]`
10|
11|| Option | Type | Default | Description |
12||---|---|---|---|
13|| `worker_threads` | integer | 0 (auto) | Tokio worker threads. 0 = all cores. Pin for fixed-shape hosts. |
14|| `ram_limit_mb` | integer | 512 | Hard RAM budget. New IPs rejected when exceeded. |
15|| `shard_count` | integer | 256 | Pre-aggregator shards (power of 2). |
16|
17|## `[detection]`
18|
19|| Option | Type | Default | Description |
20||---|---|---|---|
21|| `rps_threshold` | integer | 500 | Per-IP tripwire (requests/sec). Crossing → block. |
22|| `rate_window_secs` | integer | 10 | EWMA rate window. |
23|| `promote_min_events` | integer | 4 | Min events before an IP gets a full record. |
24|| `emergency_burst_threshold` | integer | 500 | Events/IP in one unflushed window triggers immediate block. |
25|| `batch_window_ms` | integer | 25 | Detection batch window. Lower = faster decisions. |
26|| `batch_max_events` | integer | 4096 | Max events per batch. |
27|| `pre_aggs_flush_interval_ms` | integer | 250 | Pre-aggregator flush interval. |
28|| `pre_aggs_max_size` | integer | 1000000 | Max pre-aggregation entries before forced flush. |
29|| `bloom_bits` | integer | 8000000 | Bloom filter size (bits) for promotion cache. |
30|| `subnet_batch_threshold` | integer | 64 | Distinct source IPs per /24 to trigger swarm detection. |
31|| `subnet_batch_min_events` | integer | 500 | Events per /24 in 2 s window for swarm block. |
32|| `subnet_window_threshold` | integer | 12 | Hot /24 filter floor. |
33|| `batch_block_enabled` | bool | true | Enable automatic batch blocking. |
34|| `block_ttl_secs` | integer | 300 | Default block lifetime for per-IP blocks. |
35|| `subnet_burst_ttl_secs` | integer | 60 | Shorter TTL for subnet blocks (shared egress IPs). |
36|| `pulse_window_secs` | integer | 5 | Pulse-wave detection window. |
37|| `pulse_threshold_samples` | integer | 3 | Over-threshold samples before pulse escalation. |
38|
39|## `[ipc]`
40|
41|| Option | Type | Default | Description |
42||---|---|---|---|
43|| `tcp_addr` | string | `127.0.0.1:7890` | IPC listener address. |
44|| `max_connections` | integer | 1000000 | Max concurrent IPC connections. |
45|| `max_line_length` | integer | 33554432 | Max JSON line length (bytes). |
46|| `max_connection_bytes` | integer | 1048576 | Max bytes per connection. |
47|| `read_timeout_ms` | integer | 5000 | Read timeout. |
48|| `write_timeout_ms` | integer | 5000 | Write timeout. |
49|| `connection_idle_timeout_ms` | integer | 30000 | Idle connection timeout. |
50|| `auth_keys` | string[] | [] | `key_id:hex_key` pairs. Non-empty ⇒ HMAC required on every frame. |
51|| `require_auth` | bool | false | Reject startup with empty `auth_keys`. |
52|| `behind_tls_proxy` | bool | false | Set true when a TLS proxy terminates before RamShield. |
53|
54|## `[dashboard]`
55|
56|| Option | Type | Default | Description |
57||---|---|---|---|
58|| `enabled` | bool | true | Serve dashboard UI. |
59|| `http_addr` | string | `127.0.0.1:9999` | Dashboard listener address. |
60|| `block_log_size` | integer | 1000 | Block history log size. |
61|| `admin_password_hash` | string | (none) | Argon2 PHC hash. Required for non-loopback bind. |
62|| `session_ttl_secs` | integer | 28800 | Dashboard session lifetime. |
63|| `max_login_attempts` | integer | 50 | Login lockout threshold. |
64|| `max_password_length` | integer | 1024 | Max password length for login form. |
65|| `trusted_proxies` | string[] | [] | Trusted reverse proxy IPs for client IP extraction. |
66|| `tls_enabled` | bool | false | Set true when behind HTTPS (sets `Secure` cookie flag). |
67|
68|## `[wal]`
69|
70|| Option | Type | Default | Description |
71||---|---|---|---|
72|| `dir` | string | `/var/lib/ramshield/wal` | WAL directory path. |
73|| `durability` | string | `GroupCommit` | `GroupCommit` (100 ms fsync) or `Flush` (per-write fsync). |
74|| `compress` | bool | true | zstd-compressed segments. |
75|| `retention_max_bytes` | integer | 536870912 | Max WAL size before oldest segments are deleted. |
76|
77|## `[xdp]`
78|
79|| Option | Type | Default | Description |
80||---|---|---|---|
81|| `enabled` | bool | false | Enable XDP enforcement. |
82|| `interface` | string | (required when enabled) | Network interface for XDP. |
83|| `mode` | string | (required when enabled) | XDP mode: `skb` (generic), `drv` (hardware), or `on` (automatic). |
84|
85|## Config validation
86|
87|RamShield validates config at startup:
88|
89|- Non-loopback IPC bind requires `auth_keys` or `behind_tls_proxy`
90|- Non-loopback dashboard bind requires `admin_password_hash`
91|- XDP config requires interface existence
92|- WAL directory must exist and be writable
93|- `ram_limit_mb = 0` rejected
94|- `shard_count` must be power of 2
95|
96|## Environment variable overrides
97|
98|Any config key can be overridden at runtime:
99|
100|```bash
101|export RAMSHIELD_DETECTION__RPS_THRESHOLD=1000
102|export RAMSHIELD_IPC__TCP_ADDR='0.0.0.0:7890'
103|export RAMSHIELD_IPC__AUTH_KEYS='k1:abcdef...'
104|```
105|
106|Format: `RAMSHIELD_<SECTION>__<KEY>`. Typed overrides are validated — invalid values fail startup.

## Tuning guide

### Engine (resource budget)

| Key | Default | Trade-off / When to change |
|---|---|---|
| `engine.worker_threads` | `0` (= auto / CPU count) | Set to `1` if you have >8 cores and want a dedicated batch thread instead of auto-detect. Rarely needed. |
| `engine.ram_limit_mb` | `512` | **⚙️** Lower to fit in a constrained VM (min 64 MB). Raise if store insertion returns `CapacityExceeded`. Do not exceed host RAM. |
| `engine.shard_count` | `256` | Must be a power of two. Only increase if profiling shows shard-lock contention; the cost grows ~O(N) per shard. |

### Detection tuning

| Key | Default | Trade-off / When to change |
|---|---|---|
| `detection.rps_threshold` | `500` | **⚙️** Raise if legitimate traffic exceeds threshold; lower if false blocks are frequent. |
| `detection.rate_window_secs` | `10` | Sliding window for count decay. Larger window = smoother EWMA; smaller = more responsive. |
| `detection.subnet_batch_threshold` | `50` | **⚙️** Minimum distinct source IPs per /24 before subnet block considered. Raise if you want fewer subnet blocks; lower if you want more. |
| `detection.subnet_batch_min_events` | `100` | **⚙️** /24 event volume gate (must also pass threshold). Higher = fewer subnet blocks. |
| `detection.batch_block_enabled` | `true` | Disable if you only want IP-scale blocking. |
| `detection.block_ttl_secs` | `300` | Auto-block TTL. Affects how long an IP stays blocked after threshold crossing. Lower = faster unblock; raise for long-lived attacks. |
| `detection.bloom_bits` | `8_000_000` | **⚙️** Bloom capacity (bits) — larger = lower false-positive estimate. Rule of thumb: `~20 × promoted IPs per 8 s epoch`. |
| `detection.batch_max_events` | `4096` | Max events per flush batch. Raise if you see backpressure (`events_shed_total`) under extreme load. |
| `detection.batch_window_ms` | `50` | Max wait before flushing a partial batch. Larger = more amortization; smaller = lower latency. |
| `detection.promote_min_events` | `8` | **⚙️** Hits in one window before a full `IpRecord` is stored. Lower = more IPs tracked; raise to reduce RAM. |
| `detection.subnet_window_threshold` | `500` | /24 event count that lowers promotion threshold for that subnet. Raise if /24-promoted IPs are too noisy. |
| `detection.subnet_burst_ttl_secs` | `120` | Subnet-burst block TTL. **Shorter than** `block_ttl_secs` — prevents whole shared-egress /24s staying locked out an hour. |
| `detection.pulse_window_secs` | `5` | Pulse-wave detection window. |
| `detection.pulse_threshold_samples` | `2` | Number of over-threshold samples to trigger a pulse-wave block. Raise if FPR observed. |
| `detection.emergency_burst_threshold` | `500` | **⚙️** One-IP fast-path block if this many events seen in the unflushed window. Raise if you want the fast path to fire less often. Set `0` to disable. |
| `detection.pre_aggs_max_size` | `1_000_000` | Max events in pre-aggregation buffer before flush. Raise if batches are forced early. |

### IPC tuning

| Key | Default | Trade-off / When to change |
|---|---|---|
| `ipc.tcp_addr` | `127.0.0.1:7890` | **⚠️** Change to public only if fronted with TLS + auth keys. Public bind without auth fails `validate()`. |
| `ipc.max_connections` | `256` | Raise if you get `rejected_connections` under load. |
| `ipc.max_line_length` | `33554432` (32 MB) | Max single JSON line. |
| `ipc.auth_keys` | `[]` (empty) | **⚠️** Must be `key_id:hex_key` when `require_auth = true`. Put real keys in gitignored overlay. |
| `ipc.require_auth` | `false` | **⚠️** If `true` and `auth_keys` empty → refuses to start. |
| `ipc.behind_tls_proxy` | `false` | **⚠️** Set `true` when binding non-loopback behind TLS proxy. |
| `ipc.read_timeout_ms` / `write_timeout_ms` | `5000` | Raise if clients disconnect prematurely. |
| `ipc.connection_idle_timeout_ms` | `30000` | Accepted connection idle timeout. |

### Dashboard tuning

| Key | Default | Trade-off / When to change |
|---|---|---|
| `dashboard.enabled` | `true` | Disable only if you never need a web UI. |
| `dashboard.http_addr` | `127.0.0.1:9999` | **⚠️** Public bind only with TLS front. |
| `dashboard.block_log_size` | `1000` | Ring buffer entries for `/api/history/blocks`. Raise if history scrolls out too fast. |
| `dashboard.admin_password_hash` | None | **⚠️** Argon2 PHC hash. Required for non-loopback. |
| `dashboard.session_ttl_secs` | `28800` | 8h session lifetime. |
| `dashboard.max_login_attempts` | `50` | Lockout after N failures. |
| `dashboard.trusted_proxies` | `[]` | IPs allowed to forward `X-Forwarded-For`. |
| `dashboard.tls_enabled` | `false` | **⚠️** Set `true` only when behind HTTPS reverse proxy. |

### WAL tuning

| Key | Default | Trade-off / When to change |
|---|---|---|
| `wal.enabled` | `false` | **⚙️** Enable to persist block state across restarts. |
| `wal.dir` | `/var/lib/ramshield/wal` | WAL directory. Must be writable. |
| `wal.durability` | `GroupCommit` | `GroupCommit` = fsync amortized; `Flush` = fdatasync on every block. |
| `wal.compress` | `true` | LZ4 segment compression. Disable only if CPU is saturated. |
| `wal.seg_max_bytes` | `67108864` (64 MiB) | Max segment size. |
| `wal.retention_max_bytes` | `536870912` (512 MiB) | Oldest segments deleted first. `0` = unlimited. |

### XDP tuning

| Key | Default | Trade-off / When to change |
|---|---|---|
| `xdp.enabled` | `false` | **⚙️** **Fail-closed default.** Enable only with kernel + NIC + capabilities. |
| `xdp.interface` | `eth0` | Change to `lo` for loopback/XDP-generic testing. |
| `xdp.mode` | `skb` | `skb` works on generic drivers; `drv` requires native NIC mode. |
| `xdp.build_mode` | `auto` | `auto` tries aya-build first, then clang C, then stub ELF. |

### Forecasting tuning

| Key | Default | Trade-off / When to change |
|---|---|---|
| `forecasting.enabled` | `true` | Disable if you never use anomaly-driven blocking. |
| `forecasting.ewma_alpha` | `0.3` | Level smoothing. Lower = smoother; raise if tracking fast bursts. |
| `forecasting.hw_beta` / `hw_gamma` | `0.1` / `0.1` | Trend + seasonal smoothing. |
| `forecasting.seasonality_period` | `3600` (1 hour) | Seasonality cycle in seconds. |
| `forecasting.anomaly_zscore` | `2.5` | z-score threshold for `ForecastAnomaly`. Lower = more sensitive. |
| `forecasting.min_entropy` | `2.0` | Min Shannon entropy (bits) for `EntropyAnomaly`. Lower = more sensitive. |