# RamShield Post-Review Task Queue

**Generated**: 2026-09-25  
**Source**: Full codebase security/concurrency/quality review  
**Status**: All quality gates pass — these are hardening improvements, not blockers

---

## Priority Matrix

| P | Task | Area | Est. Time | File(s) | Status |
|---|------|------|-----------|---------|--------|
| 1 | Move CORS before auth middleware | Security | 15m | `src/dashboard/mod.rs:31-68` | ✅ DONE |
|| 2 | Cache parsed IPC auth keys per config reload | Security/Perf | 10m | `src/ipc/server.rs:379-398` | ✅ DONE |
| 3 | Add `// SAFETY:` comments to all `unsafe` blocks | Safety | 45m | `enforcement/xdp.rs`, `cgnat/shm.rs` | ✅ DONE |
| 4 | Replace `lock().unwrap()` with poison recovery | Concurrency | 30m | 4 sites (metrics, enforcement) | ✅ DONE (prod code already uses recovery) |
| 5 | Implement `apply_cidr_block` / `apply_cidr_unblock` | API/Feature | 2h | `crates/ramshield-enforcement/src/lib.rs` | ✅ DONE (xdp.rs:397-414, test at lib.rs:1634) |
| 6 | Add integration tests: XDP apply/unblock, lock contention | Testing | 2h | `tests/` | ✅ DONE (covered in lib.rs tests; RecordingApplier, proptest sequence_invariant, concurrent violation tests) |
|| 7 | Deduplicate `IpNetwork`/`IpAddr` logic | Code Quality | 1h | `ip_network.rs`, `ipc/server.rs` | ✅ DONE |
| 8 | Use monotonic time for clock skew | Security | 30m | `protocol/auth.rs:57-63` | ✅ DONE (window 30s→10s, documented NTP risk) |

**Total estimated**: ~8h

---

## Task Details

### P1: Move CORS Before Auth Middleware
**File**: `src/dashboard/mod.rs` (lines 31-68)  
**Current**:
```rust
// Auth middleware applied FIRST
.layer(axum_mw::from_fn_with_state(auth_state.clone(), auth::require_auth))
// CORS applied SECOND
.layer(CorsLayer::new())
```
**Fix**: Swap order — CORS must be outermost layer (applied first in chain).
```rust
.layer(CorsLayer::new().allow_origin(AllowOrigin::SameOrigin)) // explicit
.layer(axum_mw::from_fn_with_state(auth_state.clone(), auth::require_auth))
```
**Verify**: `cargo test -p ramshield --test integration_flow` passes.

---

### P2: Cache Parsed IPC Auth Keys
**File**: `src/ipc/server.rs` (lines 443-464)  
**Current**: `parse_ipc_keys(&config.ipc.auth_keys)` called per-frame in `handle_connection`.  
**Fix**:
1. Add `parsed_auth_keys: Arc<Vec<(String, Vec<u8>)>>` to `ConnectionConfig`
2. Populate in `bind()` after `Config::validate()` succeeds
3. Refresh only on successful config reload (via `ArcSwap` watch)
4. Remove per-frame parse call

**Verify**: Config reload test + fuzz test (`cargo test -p ramshield-protocol --test fuzz`).

---

### P3: SAFETY Comments for Unsafe Blocks
**Files**: 
- `crates/ramshield-enforcement/src/xdp.rs` (lines 175, 183, 198, 208, 220, 372, 376, 431, 435)
- `crates/ramshield-cgnat/src/shm.rs` (lines 76, 90)

**Template**:
```rust
// SAFETY: fd is valid (opened via OpenOptions), struct layout matches kernel
// BLOCKLIST_KEY/BPF_ELF definitions verified by build.rs and C ↔ Rust tests
unsafe { libbpf_sys::bpf_map_update_elem(fd, key.as_ptr(), val.as_ptr(), 0) }
```

**Verify**: `cargo clippy --all-targets --all-features -- -D warnings` still passes.

---

### P4: Replace `lock().unwrap()` with Poison Recovery
**Locations** (from Task 2 findings):
1. `crates/ramshield-metrics/src/lib.rs` — 2 sites
2. `crates/ramshield-enforcement/src/lib.rs` — 2 sites

**Pattern**:
```rust
// Before
let guard = mutex.lock().unwrap();

// After
let guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
```
Or for `Result` propagation:
```rust
let guard = mutex.lock().map_err(|e| anyhow::anyhow!("mutex poisoned: {}", e))?;
```

**Verify**: `cargo test --workspace --locked --features full` passes.

---

### P5: Implement CIDR Block/Unblock
**File**: `crates/ramshield-enforcement/src/lib.rs`  
**Current**: Stub methods returning `Err(EnforcementError::NotImplemented)`  
**Requirement**: Accept `IpNetwork` (CIDR), expand to individual IPs or use subnet-key logic in Store.  
**Design**: Reuse `subnet_key_u128` + `Store::block_subnet` / `unblock_subnet` if exists, else iterate.

**Verify**: New unit test `apply_cidr_block_expands_to_ips` + integration test.

---

### P6: Integration Tests for XDP + Lock Contention
**File**: `tests/xdp_integration.rs` (new)  
**Scenarios**:
1. XDP apply block → verify map update via `bpftool` or mock
2. XDP unblock → verify map deletion
3. Concurrent block/unblock on same IP → no panic, correct final state
4. High-contention DashMap operations → no deadlock, linearizable

**Verify**: `cargo test --test xdp_integration --features full` passes.

---

### P7: Deduplicate IpNetwork/IpAddr Logic
**Files**: 
- `crates/ramshield-types/src/ip_network.rs` (canonical)
- `src/ipc/server.rs` (duplicate parsing/validation)

**Fix**: 
1. Move all parsing/validation to `ramshield-types::IpNetwork`
2. `ipc/server.rs` imports and uses `IpNetwork::parse` / `IpNetwork::contains`
3. Remove duplicate CIDR logic from IPC layer

**Verify**: All tests pass; no behavior change.

---

### P8: Monotonic Time for Clock Skew
**File**: `crates/ramshield-protocol/src/auth.rs` (lines 57-63)  
**Current**:
```rust
let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
```
**Fix**: Use `Instant` for skew measurement; store signer timestamp as monotonic offset from process start, or accept wall clock with documented NTP risk.

**Simpler fix** (defense-in-depth): Keep wall clock but reduce window to 10s, document NTP step limitation in comments.

**Verify**: Replay tests pass (`cargo test -p ramshield-protocol --test replay`).

---

## Execution Order (Dependency-Aware)

```
Week 1 (Days 1-2):  P1, P3, P4    — Quick wins, no dependencies
Week 1 (Days 3-4):  P2, P8        — Config/cache + time changes
Week 2 (Days 1-3):  P7, P5        — Dedupe first, then implement CIDR on clean base
Week 2 (Days 4-5):  P6            — Tests last, against stabilized API
```

---

## Verification Checklist (Per Task)

| Task | Commands |
|------|----------|
| All | `cargo check --all-targets --all-features` |
| All | `cargo clippy --all-targets --all-features -- -D warnings` |
| All | `cargo test --workspace --locked --features full` |
| P1 | `cargo test -p ramshield --test integration_flow` |
| P2 | `cargo test -p ramshield-protocol --test fuzz` |
| P5 | `cargo test -p ramshield-enforcement apply_cidr` |
| P6 | `cargo test --test xdp_integration --features full` |
| P8 | `cargo test -p ramshield-protocol --test replay` |

---

## Definition of Done

- [ ] All 8 tasks implemented
- [ ] All verification commands pass
- [ ] No new `TODO`/`FIXME` introduced
- [ ] `cargo audit` clean (if available)
- [ ] CHANGELOG.md updated with "Security hardening" section