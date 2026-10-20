# RamShield IPC Protocol

Wire format: newline-delimited JSON over TCP. One JSON `Request` per line, one JSON `Response` per line. Server is `src/ipc/server.rs::IpcServer`; framing handled by `handle_connection` (`src/ipc/server/connection.rs`).

Transport: TCP, address from `ipc.tcp_addr`, max concurrent connections from `ipc.max_connections`.

## Authentication (P1/P2/P3)

### Default posture

**The out-of-the-box IPC is unauthenticated.** `ipc.auth_keys` defaults to
empty and `ipc.require_auth` defaults to `false`
(`crates/ramshield-config/src/sections/ipc.rs`), so with the default
configuration the server accepts frames that carry no `auth` envelope at all:

- there is no principal, so the "Min role" column in the request table below
  is **not** applied — role authorization only runs when a principal is
  present (`src/ipc/server/handlers.rs`, `process_request`);
- there is no verified key, so the enforcement command's `actor` falls back
  to the historical hard-coded `"admin"`.

The bind defaults to `127.0.0.1:7890`, so this is a **loopback-only** surface:
any local process on the host can drive it. Widening the exposure is
prevented at startup — a non-loopback `ipc.tcp_addr` fails `validate()` unless
`ipc.auth_keys` is non-empty *and* `ipc.behind_tls_proxy = true` is asserted
(`crates/ramshield-config/src/validate.rs`).

Everything below this paragraph describes the **authenticated** mode, which is
what you get once `ipc.auth_keys` is configured.

### Authenticated mode

When `ipc.auth_keys` is non-empty, every frame MUST carry an `auth` envelope:

```json
{"auth":{"key_id":"k1","ts_ms":1696000000000,"sig":"<hex>"},"type":"block_ip","ip":"1.2.3.4","reason":"manual","ttl_secs":300}
```

- **HMAC-SHA256** over `<ts_ms>.<key_id><payload>` (payload = compact JSON of the frame without `auth`).
- **Clock skew**: ±10 s (`MAX_CLOCK_SKEW_MS`).
- **Replay protection**: mandatory in production — `verify_authenticated` requires a `&ReplayStore`. The server constructs one bounded LRU store at startup; production IPC never passes `None`.
- **Identity (P1)**: the verified `key_id` becomes the `actor` field on every enforcement command — no hard-coded `"admin"`. (This holds only in authenticated mode; see *Default posture* above.)
- **Authorization (P2)**: in authenticated mode, `key_roles` config assigns a `KeyRole` per `key_id`. Roles: `Telemetry` < `ReadOnly` < `Operator` < `Admin`. Unauthorized requests return `403 "insufficient role"`. Unauthenticated frames return `401`.
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

The **Min role** column applies only when a principal is present — that is,
when `ipc.auth_keys` is non-empty. In the default unauthenticated mode no
role is checked. See *Default posture* above.

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
key = bytes.fromhex('ReplaceWithYourOwn256BitKeyInHex')
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
- Authorization: `src/ipc/server.rs` (`Principal`) and `src/ipc/server/auth.rs` (`authorize`, `verify_frame_auth`)
- Config: `crates/ramshield-config/src/sections/ipc.rs` (`IpcConfig`, `KeyRole`, `KeyRoleConfig`)
- Server / framing: `src/ipc/server.rs` (`IpcServer`), `src/ipc/server/connection.rs` (`handle_connection`), `src/ipc/server/handlers.rs` (`process_request`)
