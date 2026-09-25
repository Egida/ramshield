# RamShield IPC Protocol

Wire format: newline-delimited JSON over TCP. One JSON `Request` per line, one JSON `Response` per line. Server is `src/ipc/server.rs::IpcServer`; framing handled by `handle_connection`.

Transport: TCP, address from `ipc.tcp_addr`, max concurrent connections from `ipc.max_connections`.

## Authentication (P1/P2/P3)

When `ipc.auth_keys` is non-empty, every frame MUST carry an `auth` envelope:

```json
{"auth":{"key_id":"k1","ts_ms":1696000000000,"sig":"<hex>"},"type":"block_ip","ip":"1.2.3.4","reason":"manual","ttl_secs":300}
```

- **HMAC-SHA256** over `<ts_ms>.<key_id><payload>` (payload = compact JSON of the frame without `auth`).
- **Clock skew**: ±10 s (`MAX_CLOCK_SKEW_MS`).
- **Replay protection**: mandatory in production — `verify_authenticated` requires a `&ReplayStore`. The server constructs one bounded LRU store at startup; production IPC never passes `None`.
- **Identity (P1)**: the verified `key_id` becomes the `actor` field on every enforcement command — no hard-coded `"admin"`.
- **Authorization (P2)**: `key_roles` config assigns a `KeyRole` per `key_id`. Roles: `Telemetry` < `ReadOnly` < `Operator` < `Admin`. Unauthorized requests return `403 "insufficient role"`. Unauthenticated frames return `401`.
- **Transport safety (P4)**: a non-loopback `ipc.tcp_addr` without `ipc.behind_tls_proxy = true` fails `validate()` at startup. HMAC authenticates; it does not encrypt.

## Requests (`tag = "type"`, snake_case)

| Type | Fields | Min role |
|------|--------|----------|
| `check_ip`           | `ip: string` | ReadOnly |
| `block_ip`           | `ip: string`, `reason: string`, `ttl_secs: number \| null` | Operator |
| `block_cidr`         | `cidr: string`, `reason: string`, `ttl_secs: number \| null` | Operator |
| `unblock_ip`         | `ip: string` | Operator |
| `unblock_cidr`       | `cidr: string` | Operator |
| `get_ip_stats`       | `ip: string` | ReadOnly |
| `get_stats`          | — | ReadOnly |
| `get_status`         | — | ReadOnly |
| `report_connection`  | `ip: string`, `bytes: number`, `status_code: number`, `proto_fp: number` | Telemetry |
| `report_connections` | `events: [{ ip, bytes, status_code, proto_fp }, ...]` | Telemetry |
| `flush`              | — | Admin |

## Responses

| Type | Fields |
|------|--------|
| `ip_status`  | `ip`, `blocked`, `threat`, `ewma_rps`, `reason` |
| `ok`         | `message` |
| `batch_ok`   | `accepted`, `rejected` |
| `error`      | `code` (4xx/5xx), `message` |
| `stats`      | `ips_tracked`, `blocked`, `ram_bytes`, `ram_limit_mb`, `uptime_secs`, `evictions` |
| `ip_detail`  | `ip`, `count`, `ewma_rps`, `threat`, `state`, `bytes_in`, `first_seen_s`, `last_seen_s` |

## Errors

| Condition | Code | Message |
|-----------|------|---------|
| Invalid JSON | 400 | `<serde err>` |
| Invalid IP / CIDR | 400 | `invalid ip address: <ip>` / `invalid CIDR: <cidr>` |
| TTL overflow | 400 | `ttl_secs exceeds 1 year` |
| Unauthenticated frame (auth_keys set) | 401 | `missing auth object` |
| Insufficient role | 403 | `insufficient role` |
| Enforcement queue full (P8) | 503 | `enforcement queue full` |
| Serialise failure | 500 | `serialise failed` |

## Minimal example (signed frame, POSIX + python3)

```sh
# Sign and send a block_ip request with key k1
python3 -c "
import hmac, hashlib, time, json, socket, sys
key = bytes.fromhex('0b8d647fda3a0ae3c38207e0d7e61edfdfe59bda7359c89f953f76ed68f3768b')
ts = int(time.time() * 1000)
req = {'type':'check_ip','ip':'1.2.3.4'}
payload = json.dumps(req, separators=(',',':'), sort_keys=True).encode()
sig = hmac.new(key, f'{ts}.k1'.encode() + payload, hashlib.sha256).hexdigest()
frame = {'auth':{'key_id':'k1','ts_ms':ts,'sig':sig}, **req}
s = socket.socket(); s.connect(('127.0.0.1', 7890))
s.sendall((json.dumps(frame, separators=(',',':'), sort_keys=True) + '\n').encode())
print(s.recv(4096).decode().strip()); s.close()
"
```

## Source of truth

- Types: `crates/ramshield-protocol/src/message.rs`
- Auth + replay: `crates/ramshield-protocol/src/auth.rs` (`verify`, `verify_authenticated`, `ReplayStore`)
- Authorization: `src/ipc/server.rs` (`Principal`, `authorize`)
- Config: `crates/ramshield-config/src/lib.rs` (`IpcConfig`, `KeyRole`, `KeyRoleConfig`)
- Server / framing: `src/ipc/server.rs` (`IpcServer`, `handle_connection`, `process_request`)
