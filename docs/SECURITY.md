# RamShield Security

## Trust Model

RamShield assumes:
1. The Linux host is controlled by the operator (kernel, BPF subsystem, filesystem).
2. The proxy (Nginx/HAProxy) sending telemetry is trusted to produce truthful ConnectionReports.
3. The IPC network is a trusted loopback or TLS-proxied path.
4. The dashboard is accessed by authorized operators only.

RamShield does **not** protect against:
- A compromised host (attacker with root can disable XDP, truncate WAL, stop the daemon).
- A compromised proxy sending malicious telemetry (can cause false blocks).
- Cryptographic attacks on HMAC-SHA256 or Argon2id (standard primitives, no known practical weakness).

## Security Boundaries

### IPC (port 7890)

| Property | Detail |
|---|---|
| Default bind | `127.0.0.1:7890` (loopback only) |
| Authentication | HMAC-SHA256 envelope over each JSON frame. Key material is `key_id:hex_key` pairs in config. |
| Authorization | Roles: Telemetry, ReadOnly, Operator, Admin. Admin may flush/change config. |
| Replay protection | Per-key LRU nonce store (1024 entries, 65s TTL). Frames with timestamps outside ±30s clock skew are rejected. |
| Transport security | **None.** HMAC authenticates but does not encrypt. Public bind requires `behind_tls_proxy=true` and a TLS-terminating proxy in front. |
| Connection limits | Configurable `max_connections` (default 256), `max_connection_bytes` (default 1 MB), `max_line_length` (default 1 MB). |
| Fail-closed | `ipc.require_auth=true` with empty `auth_keys` prevents startup. Public bind without keys prevents startup. |

### Dashboard (port 9999)

| Property | Detail |
|---|---|
| Default bind | `127.0.0.1:9999` (loopback only) |
| Authentication | Argon2id password hash in config session, HMAC-signed cookies. |
| Session lifetime | Configurable (default 8h). |
| Brute-force protection | Per-IP lockout after configurable attempts (default 50), with trusted proxy support for X-Forwarded-For. |
| CSRF protection | Origin/Referer header check on POST /api/config. SameSite=Lax on session cookie. |
| Transport security | **None.** Dashboard is HTTP only. `Secure` cookies work only on loopback (RFC 6265bis). Public bind requires TLS reverse proxy. |
| Key material exposure | GET/POST /api/config redacts HMAC keys (`<redacted>`) and password hash (`<redacted>`). Placeholder rejection prevents POST-back of a viewed config from silently disabling auth. |

### WAL Files

| Property | Detail |
|---|---|
| Permissions | `0o600`, owned by `ramshield` user. |
| Integrity | CRC checksums on every record. Decompression-bomb guard. |
| Retention | Segment rotation bounded by `retention_max_bytes` (default 512 MB). Oldest segments deleted first. |

### XDP / BPF Maps

| Property | Detail |
|---|---|
| Access | Kernel-protected. Only the owning process can update. |
| Capacity | Per-IP maps: LRU with self-eviction. CIDR LPM trie: 102,400 entries (no LRU). |
| Drift detection | Reconciliation loop compares store blocks vs kernel maps every 250ms. Exposed as `xdp_reconcile_age_seconds` metric. |
| Stale entry cleanup | Reconciliation removes stale entries not in the store. |

## Operational Security

### Capabilities

RamShield requires:
- `CAP_BPF` — BPF map creation and program attachment
- `CAP_NET_ADMIN` — XDP program management
- `CAP_PERFMON` — BPF observability stat collection

Set via: `setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' /usr/bin/ramshield`

### Service Isolation

Run as dedicated `ramshield` user with:
- No shell access
- Write access only to WAL directory (`/var/lib/ramshield/wal`)
- Read-only access to config (`/etc/ramshield/config.toml`)
- Network bind allowed only to configured IPC/dashboard ports via `AmbientCapabilities`

### Secret Rotation

1. IPC HMAC keys: update `ipc.auth_keys` in config and restart. Clients reconnecting with old keys receive 401.
2. Dashboard password: POST to /api/config or set `RAMSHIELD_DASHBOARD__ADMIN_PASSWORD_HASH` env var. Hot-reloads the session validator.

## Dependency Security

All dependencies are audited before release:
- Rust crate dependencies tracked in `Cargo.lock`
- `cargo audit` runs in CI
- No untrusted build-time code execution
- No build-downloaded binaries (bpf-linker is OS-packaged or prebuilt and tracked)

## Vulnerability Reporting

Contact the maintainer directly. No bug bounty program currently exists.