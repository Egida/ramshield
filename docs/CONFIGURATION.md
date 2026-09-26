# Configuration

The configuration is TOML. Environment variables can override config values using:

```text
RAMSHIELD_<SECTION>__<KEY>
```

The canonical repository example is [`config.baseline.toml`](../config.baseline.toml). The schema and validation live in [`crates/ramshield-config/src/lib.rs`](../crates/ramshield-config/src/lib.rs).

The baseline file intentionally contains explicit values for development/testing. It is not a universal production tuning profile.

## `[engine]`

| Key | Baseline | Built-in default | Notes |
|---|---:|---:|---|
| `worker_threads` | `0` | `0` | `0` lets Tokio choose worker count. |
| `ram_limit_mb` | `14512` | `512` | Hard store budget; validation requires at least 64 MB. |
| `shard_count` | `256` | `256` | Must be a power of two. |

## `[detection]`

| Key | Baseline | Built-in default |
|---|---:|---:|
| `rps_threshold` | `500` | `1000` |
| `rate_window_secs` | `10` | `10` |
| `promote_min_events` | `4` | `8` |
| `emergency_burst_threshold` | `500` | `500` |
| `batch_window_ms` | `25` | `50` |
| `batch_max_events` | `4096` | `4096` |
| `pre_aggs_flush_interval_ms` | `250` | `1000` |
| `pre_aggs_max_size` | `1000000` | `1000000` |
| `bloom_bits` | `8000000` | `8000000` |
| `subnet_batch_threshold` | `64` | `50` |
| `subnet_batch_min_events` | `500` | `100` |
| `subnet_window_threshold` | `12` | `500` |
| `batch_block_enabled` | `true` | `true` |
| `block_ttl_secs` | `300` | `3600` |
| `subnet_burst_ttl_secs` | `60` | `120` |
| `pulse_window_secs` | `5` | `6` |
| `pulse_threshold_samples` | `3` | `2` |

Use the baseline values only when they are appropriate for your workload. Detection thresholds are policy, not universal safe values.

## `[ipc]`

| Key | Baseline | Built-in default |
|---|---|---:|
| `tcp_addr` | `127.0.0.1:7890` | `127.0.0.1:7890` |
| `max_connections` | `1000000` | `256` |
| `max_line_length` | `33554432` | 32 MiB when serde default is applied |
| `max_connection_bytes` | `1048576` | 1 MiB |
| `read_timeout_ms` | `5000` | 5000 |
| `write_timeout_ms` | `5000` | 5000 |
| `connection_idle_timeout_ms` | `30000` | 30000 |
| `auth_keys` | `[]` | `[]` |
| `require_auth` | `false` | `false` |

For a non-loopback IPC bind, configuration validation requires authentication and also requires `behind_tls_proxy = true`.

## `[dashboard]`

| Key | Baseline | Built-in default |
|---|---:|---:|
| `enabled` | `true` | `false` |
| `http_addr` | `127.0.0.1:9999` | `127.0.0.1:9999` |
| `block_log_size` | `1000` | `1000` |
| `admin_password_hash` | unset | unset |
| `session_ttl_secs` | `28800` | `28800` |
| `max_login_attempts` | `50` | `50` |
| `max_password_length` | `1024` | `1024` |
| `trusted_proxies` | `[]` | `[]` |
| `tls_enabled` | `false` | `false` |
| `cookie_secure` | unset | unset |

A non-loopback dashboard bind requires `admin_password_hash`.

RamShield has no built-in TLS listener. `tls_enabled` controls cookie behavior for an external HTTPS termination boundary; it does not enable a TLS server.

## `[wal]`

| Key | Baseline | Built-in default |
|---|---:|---:|
| `enabled` | `true` | `false` |
| `dir` | `/var/lib/ramshield/wal` | `/var/lib/ramshield/wal` |
| `durability` | `GroupCommit` | `Flush` |
| `compress` | `true` | `true` |
| `seg_max_bytes` | `67108864` | `67108864` |
| `retention_max_bytes` | `1073741824` | `536870912` |

WAL is what provides block-state persistence across restarts. Keep its directory writable and protected.

## `[xdp]`

| Key | Baseline | Built-in default |
|---|---|---|
| `enabled` | `true` | `false` |
| `interface` | `eth0` | `eth0` |
| `mode` | `skb` | `skb` |

Valid modes are the values currently accepted by the XDP implementation, including `skb` and `drv`.

## `[forecasting]`

| Key | Baseline | Built-in default |
|---|---:|---:|
| `enabled` | `true` | `true` |
| `ewma_alpha` | `0.3` | `0.3` |
| `hw_beta` | `0.1` | `0.1` |
| `hw_gamma` | `0.1` | `0.1` |
| `seasonality_period` | `60` | `3600` |
| `anomaly_zscore` | `3.0` | `2.5` |
| `min_entropy` | `4.5` | `2.0` |

## Environment overrides

Example:

```bash
export RAMSHIELD_DETECTION__RPS_THRESHOLD=1000
export RAMSHIELD_IPC__TCP_ADDR='127.0.0.1:7890'
```

Typed values are validated.

## Validation behavior

Validation happens as part of configuration loading/startup. Important current guards include:

- `engine.ram_limit_mb >= 64`;
- `engine.shard_count` is a non-zero power of two;
- positive detection thresholds and valid batch bounds;
- public dashboard binds require an admin password hash;
- public IPC binds require auth keys and a TLS proxy boundary;
- `require_auth = true` requires at least one auth key.

There is currently no standalone `ramshield config validate` CLI command. Invalid configuration should therefore be treated as a startup failure.

## Secrets

Do not commit:

- IPC HMAC keys;
- dashboard password hashes you intend to keep private;
- other deployment secrets.

Use an ignored overlay or environment variables.