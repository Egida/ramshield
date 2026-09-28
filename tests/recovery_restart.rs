//! Recovery restart integration tests.
//!
//! Each test simulates a crash/restart cycle: write state, "crash" (drop WAL),
//! reopen WAL, replay into fresh store, and verify recovered state.
//!
//! Uses full replay (min_lsn=0) — validates that WAL files persistently
//! capture block state and replay restores it correctly.  This is the
//! same path as no-snapshot recovery.
//!
//! Test scenarios (review PATCH 28):
//!   1. permanent IP — block, crash, restart → blocked
//!   2. temporary IP with active TTL — block ttl=10, restart → blocked + TTL
//!   3. expired before restart — block ttl=1, advance clock 5s, restart → unblocked
//!   4. CIDR block — block /24 ttl=20, crash, restart → /24 active with TTL
//!   5. tail replay — append entries before and after checkpoint, restart → both present

use ramshield_enforcement::{replay_wal_cidrs_from, replay_wal_into_store};
use ramshield_storage::{Store, wal::Wal, wal::WalEntry};
use ramshield_types::{Durability, IpNetwork};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn wal_dir(label: &str) -> String {
    let d = std::env::temp_dir().join(format!("rs_recovery_{}_{}", label, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_str().unwrap().to_string()
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Fresh store + reopen WAL from dir, replay all entries.
fn restart_full_replay(dir: &str) -> (Arc<Store>, Vec<(IpAddr, u64)>) {
    let store = Arc::new(Store::new(16));
    store
        .traffic
        .ram_limit_mb
        .store(256, std::sync::atomic::Ordering::Relaxed);
    let wal = Wal::open(dir, false, Durability::None, 64 * 1024 * 1024, 0).unwrap();
    let ttls = replay_wal_into_store(&store, &wal, 0).unwrap();
    drop(wal);
    (store, ttls)
}

// ── Scenario 1: Permanent IP ──────────────────────────────────────────

#[test]
fn recovery_permanent_ip() {
    let dir = wal_dir("perm_ip");
    let wal = Wal::open(&dir, false, Durability::None, 64 * 1024 * 1024, 0).unwrap();
    let ip = IpAddr::from([10, 0, 0, 1]);

    wal.append(&WalEntry::BlockIp {
        ip: ip.to_string(),
        reason: "test".into(),
        ttl_secs: None,
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);

    let (store, _) = restart_full_replay(&dir);
    let rec = store.get(&ip).unwrap();
    assert!(rec.is_blocked(), "permanent block must survive restart");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Scenario 2: Temporary IP with active TTL ──────────────────────────

#[test]
fn recovery_temporary_ip_ttl_active() {
    let dir = wal_dir("tmp_ip_ttl");
    let wal = Wal::open(&dir, false, Durability::None, 64 * 1024 * 1024, 0).unwrap();
    let ip = IpAddr::from([10, 0, 0, 2]);

    wal.append(&WalEntry::BlockIp {
        ip: ip.to_string(),
        reason: "test".into(),
        ttl_secs: Some(10),
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);

    let (store, ttls) = restart_full_replay(&dir);

    let rec = store.get(&ip).unwrap();
    assert!(rec.is_blocked(), "IP must be blocked after restart");

    let remaining = ttls
        .iter()
        .find(|(addr, _)| *addr == ip)
        .map(|(_, s)| *s)
        .unwrap_or(0);
    assert!(
        remaining > 0 && remaining <= 10,
        "TTL plausible (1-10s), got {remaining}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Scenario 3: Expired before restart ────────────────────────────────

#[test]
fn recovery_expired_before_restart() {
    let dir = wal_dir("expired");
    let wal = Wal::open(&dir, false, Durability::None, 64 * 1024 * 1024, 0).unwrap();
    let ip = IpAddr::from([10, 0, 0, 3]);

    // Append a block that expired 5s ago (TTL=1s).
    let past_ts = now_ns() - 5_000_000_000;
    wal.append(&WalEntry::BlockIp {
        ip: ip.to_string(),
        reason: "test".into(),
        ttl_secs: Some(1),
        ts_ns: past_ts,
    })
    .unwrap();
    drop(wal);

    let (store, ttls) = restart_full_replay(&dir);

    let rec = store.get(&ip);
    let blocked = rec.map(|v| v.is_blocked()).unwrap_or(false);
    assert!(!blocked, "expired block must not survive restart");

    let ip_ttl = ttls.iter().find(|(addr, _)| *addr == ip);
    assert!(ip_ttl.is_none(), "expired TTL must not be restored");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Scenario 4: CIDR block with TTL ───────────────────────────────────

#[test]
fn recovery_cidr_block() {
    let dir = wal_dir("cidr");
    let wal = Wal::open(&dir, false, Durability::None, 64 * 1024 * 1024, 0).unwrap();
    let net = IpNetwork::new(IpAddr::from([10, 0, 0, 0]), 24).unwrap();

    wal.append(&WalEntry::BlockCidr {
        cidr: net,
        reason: "test".into(),
        ttl_secs: Some(20),
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);

    let wal2 = Wal::open(&dir, false, Durability::None, 64 * 1024 * 1024, 0).unwrap();
    let cidrs = replay_wal_cidrs_from(&wal2, 0).unwrap();
    drop(wal2);

    assert_eq!(cidrs.len(), 1, "one CIDR must be restored");
    let (restored_net, remaining) = cidrs[0];
    assert_eq!(restored_net, net, "CIDR network must match");
    assert!(
        remaining > 0 && remaining <= 20,
        "TTL plausible (1-20s), got {remaining}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Scenario 5: Tail replay (checkpoint → append more → full replay) ──

#[test]
fn recovery_tail_replay() {
    let dir = wal_dir("tail");
    let wal = Wal::open(&dir, false, Durability::None, 64 * 1024 * 1024, 0).unwrap();
    let ip_a = IpAddr::from([10, 0, 1, 1]);
    let ip_b = IpAddr::from([10, 0, 1, 2]);

    // Entry before checkpoint.
    wal.append(&WalEntry::BlockIp {
        ip: ip_a.to_string(),
        reason: "pre_ckpt".into(),
        ttl_secs: None,
        ts_ns: now_ns(),
    })
    .unwrap();

    // Checkpoint (writes manifest).
    let boundary = wal.begin_checkpoint();
    wal.finish_checkpoint(boundary.lsn, "").unwrap();

    // Entry after checkpoint (tail).
    wal.append(&WalEntry::BlockIp {
        ip: ip_b.to_string(),
        reason: "post_ckpt".into(),
        ttl_secs: None,
        ts_ns: now_ns(),
    })
    .unwrap();

    drop(wal);

    let (store, _) = restart_full_replay(&dir);

    let rec_a = store.get(&ip_a).unwrap();
    assert!(rec_a.is_blocked(), "pre-checkpoint block must survive");
    let rec_b = store.get(&ip_b).unwrap();
    assert!(
        rec_b.is_blocked(),
        "post-checkpoint (tail) block must survive"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
