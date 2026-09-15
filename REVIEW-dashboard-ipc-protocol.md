# Dashboard + IPC Protocol Deep Review — 2026-09-15

## Summary

| # | Severity | File:Line | Issue |
|---|----------|-----------|-------|
| 1 | **HIGH** | `src/dashboard/auth.rs:317` | Secure cookie on plain HTTP — login impossible |
| 2 | MEDIUM | `src/dashboard/static/index.html:529-538` | loadBlocks() silently swallows auth failures |
| 3 | MEDIUM | `src/ipc/server.rs:387-396` | parse_ipc_keys failure: frame silently dropped, no error response |
| 4 | MEDIUM | `src/cli.rs:75` | Hardcoded `key_id = "k1"` — multi-key broken |
| 5 | LOW | `src/dashboard/mod.rs:334,388` | `hlc_drift_ms` hardcoded 0 — KPI "Cluster Clock Drift" always 0 ms |
| 6 | LOW | `src/dashboard/static/index.html:483,500,503` | od-capex / od-cgblock / od-cghit CSS class never resets to normal |
| 7 | LOW | `src/dashboard/static/index.html:424-426` | HostBitmap matrix is cosmetic — not real bitmap data |
| 8 | LOW | `src/ipc/server.rs:187-199` | ReplayStore LRU eviction enables replay under >1024-frame flood |
| 9 | LOW | `src/dashboard/mod.rs:36-82,106` | TelemetryCollector dead code (never instantiated) |
| 10 | LOW | `src/dashboard/auth.rs:207` | Auth exempts `/static/` but no static route exists |
| 11 | LOW | `crates/ramshield-protocol/src/message.rs:7-35` | `Message`/`Body` types dead — wire uses flat Request/Response |
| 12 | LOW | `crates/ramshield-protocol/README.md` | Protocol README completely stale (wrong envelope, sign(), ReplayStore) |
| 13 | LOW | `crates/dashboard/` (entire dir) | Orphaned crate, not in workspace, broken imports |
| 14 | LOW | `crates/dashboard/static/index.html:35` | fetch fallback ignores HTTP status (dead crate) |
| 15 | LOW | `src/cli.rs:73` | Auth canonicalization coupling implicit (Value → BTreeMap sort) |

---

## Detailed Findings

### 1. Login cookie `Secure` flag breaks plain HTTP auth (HIGH)

**File:** `src/dashboard/auth.rs:317`

```rust
let cookie = format!(
    "{}={}; HttpOnly; SameSite=Lax; Secure; Path=/; Max-Age={}",
    COOKIE_NAME, token, auth.ttl.as_secs()
);
```

Dashboard binds plain HTTP (no TLS — `P1-7` warning acknowledged in config). When `admin_password_hash` is set (auth enabled), the browser refuses to store/send a `Secure` cookie over `http://`. User loops back to `/login` forever. The `Secure` flag is unconditional — no TLS presence check.

**Fix:** Omit `Secure` when serving plain HTTP, or derive it from config:
```rust
let secure = if self.tls_enabled { "; Secure" } else { "" };
```

---

### 2. loadBlocks() swallows auth failures silently (MEDIUM)

**File:** `src/dashboard/static/index.html:529-538`

```js
async function loadBlocks() {
    try {
        let res = await fetch('/api/blocks/active');
        if (!res.ok) res = await fetch('/api/history/blocks');
        if (!res.ok) throw new Error('blocks endpoint failed');
        const data = await res.json();
        // ...
    } catch (_) {}
}
```

When session expires: both fetches return 401 → `throw` → `catch(_){}` → table stuck at "Listening for live telemetry..." with zero user feedback. User never sees an error message or re-login prompt.

**Fix:** On 401, redirect to `/login` or show inline auth expiry message.

---

### 3. parse_ipc_keys failure: silent frame drop (MEDIUM)

**File:** `src/ipc/server.rs:387-396`

```rust
let live_keys = {
    let cfg = config.config.load();
    match parse_ipc_keys(&cfg) {
        Ok(k) => k,
        Err(e) => {
            debug!("parse_ipc_keys failed: {e}; rejecting frame");
            continue;  // ← no error response sent
        }
    }
};
```

If hot-reloaded config produces malformed `auth_keys`, `parse_ipc_keys` errors → frame silently dropped, no response written. Client hangs until read timeout (5s default), not getting any error code. Also: `debug!` level means no operator visibility in production (info/warn).

**Fix:** Send a 500 error frame before `continue`; upgrade log to `warn!`.

---

### 4. CLI hardcodes `key_id = "k1"` (MEDIUM)

**File:** `src/cli.rs:75`

```rust
let key_id = "k1"; // per config; for now static
```

If server has `k2`, `admin`, or any non-`k1` key configured, CLI auth always fails with "unknown key_id". Not read from config or CLI args.

**Fix:** Add `--key-id` CLI flag, defaulting to `k1`.

---

### 5. hlc_drift_ms hardcoded 0 (LOW)

**File:** `src/dashboard/mod.rs:334`

```rust
"hlc_drift_ms": 0, // single-node without mesh: HLC is not a drift source
```

KPI "Cluster Clock Drift" always shows "0 ms". Also `mesh.hlc_drift_ms` hardcoded 0 at line 388. Intentional for single-node, but multi-node deployment gets wrong data.

---

### 6. CSS class latch bug (LOW)

**File:** `src/dashboard/static/index.html:483,500,503`

```js
// od-capex: once crit, never resets
if (capEx > 0) $('od-capex').className = 'ops-value crit';
// od-cgblock: same pattern
if ((cg.tier_block || 0) > 0) $('od-cgblock').className = 'ops-value crit';
// od-cghit: same pattern
if ((cg.shm_hit_rate_pct || 100) < 50) $('od-cghit').className = 'ops-value warn';
```

Unlike `od-ram` which has a full if/else-if chain, these three elements stick at crit/warn even after value returns to normal.

**Fix:** Add reset clause: `else $('od-capex').className = 'ops-value';`

---

### 7. HostBitmap matrix is cosmetic (LOW)

**File:** `src/dashboard/static/index.html:424-426`

```js
const activeBits = Math.min(256, d.subnet_bitmap_ones || 0);
// ... fills first N cells with fake alternating colors
cells[i].classList.add(i % 5 === 0 ? 'tier-block' : 'tier-pass');
```

`subnet_bitmap_ones` = `stats.ips_tracked` (tracked IP count), not actual HostBitmap popcnt. Matrix renders a fake progress bar, not real subnet density. Panel title says "HostBitmap Subnet Density (/24 Popcnt)" but shows something else entirely.

---

### 8. ReplayStore LRU eviction enables replay (LOW)

**File:** `src/ipc/server.rs:187-199`

ReplayStore capacity = 1024, TTL = 65s. Under >1024 distinct valid frames within 65s (easy under 1M eps), LRU evicts old nonces → previously-seen frame's nonce reappears → replay accepted. Comment acknowledges "brief hole". Practical risk: attacker must have a legitimate key and flood within the window.

---

### 9-10. Dead code: TelemetryCollector + /static/ exemption

**File:** `src/dashboard/mod.rs:36-82` — `TelemetryCollector` defined but `AppState.collector` is always `None` (line 106). Never used.

**File:** `src/dashboard/auth.rs:207` — `path.starts_with("/static/")` exempted from auth, but no static route exists in the live router. Dead path.

---

### 11. Message/Body types dead, README stale

**File:** `crates/ramshield-protocol/src/message.rs:7-35`

`Message{version, body}` and `Body` enum exported via `pub use message::*` but never used by server or CLI. IPC server parses `Request` directly (server.rs:404). Dead types.

**File:** `crates/ramshield-protocol/README.md`

Documents a completely different protocol:
- Envelope: `Message{version, auth, body}` → actual: flat `{"type":"...", "auth":{...}}`
- sign(): `HMAC-SHA256(key, "{ts_ms}:{payload}")` → actual: `ts_ms + "." + key_id + payload`
- ConnectionReport fields wrong (has `upstream_ip`, `response_time_ms`, `protocol`)
- ReplayStore: DashMap, nonce, 5min TTL → actual: Mutex+VecDeque LRU, HMAC digest, 65s
- Stats: `{blocked_ips, total_events, rps, uptime_secs}` → actual: `{ips_tracked, blocked, ram_bytes, ram_limit_mb, uptime_secs, evictions}`

---

### 13. Orphaned crates/dashboard/ (LOW)

Not in workspace members (`Cargo.toml` line 2-16). Broken imports in `crates/dashboard/src/collector.rs` (`crate::metrics`, `crate::storage`, `crate::mesh`). Cannot compile. Two complete dashboard implementations coexist confusingly.

---

## IPC Protocol Assessment

**Wire format:** Newline-delimited JSON ✓. `Request` internally-tagged by `"type"`, `deny_unknown_fields` ✓. Response same. Framing robust (BytesMut split_to O(1), read loop handles partial frames).

**Auth:** HMAC-SHA256 over `ts_ms.key_id + payload` ✓. Constant-time compare ✓. Key_id bound into MAC ✓. Live config reload ✓. fail-closed when `parse_ipc_keys` errors ✓.

**Replay:** ReplayStore checks AFTER constant-time compare (timing-safe) ✓. TTL = 2×skew + 5s slack ✓. LRU bounded ✓.

**Envelope canonicalization:** Both CLI and server serialize through `serde_json::Value` (BTreeMap keys) — signatures match ✓. Implicit coupling: client must canonicalize via Value; raw bytes won't work.

---

## What's NOT Broken

- SSE stream endpoint correct for live dashboard (`src/dashboard/mod.rs:151-406`): all nested groups, flat aliases, EWMA-smoothed rates, system metrics ✓
- All frontend `document.getElementById` calls match HTML elements ✓
- CSRF mitigation on `/api/mitigate/*` and `/api/config` POST ✓
- Session auth with per-IP lockout, Argon2 verify on blocking thread ✓
- IPC batch ingestion shedding at 75% watermark ✓
- TTL clamping prevents `u64::MAX` overflow panic ✓
- `deny_unknown_fields` on Request catches typos ✓
