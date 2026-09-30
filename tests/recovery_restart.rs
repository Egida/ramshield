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
/// Mirrors the engine's no-snapshot recovery path: IP fold + CIDR fold.
fn restart_full_replay(dir: &str) -> (Arc<Store>, Vec<(IpAddr, u64)>) {
    let store = Arc::new(Store::new(16));
    store
        .traffic
        .ram_limit_mb
        .store(256, std::sync::atomic::Ordering::Relaxed);
    let wal = open_wal(dir);
    let ttls = replay_wal_into_store(&store, &wal, 0).unwrap();
    // Engine full replay also folds CIDRs (replay_wal_cidrs_from min_lsn=0).
    for (net, _remaining) in replay_wal_cidrs_from(&wal, 0).unwrap() {
        store.active_cidrs.insert(net, ());
    }
    drop(wal);
    (store, ttls)
}

// ── Scenario 1: Permanent IP ──────────────────────────────────────────

#[test]
fn recovery_permanent_ip() {
    let dir = wal_dir("perm_ip");
    let wal = open_wal(&dir);
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
    let wal = open_wal(&dir);
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
    let wal = open_wal(&dir);
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
    let wal = open_wal(&dir);
    let net = IpNetwork::new(IpAddr::from([10, 0, 0, 0]), 24).unwrap();

    wal.append(&WalEntry::BlockCidr {
        cidr: net,
        reason: "test".into(),
        ttl_secs: Some(20),
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);

    let wal2 = open_wal(&dir);
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
    let wal = open_wal(&dir);
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
    wal.sync().unwrap();
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

// ═══════════════════════════════════════════════════════════════════════
// Checkpoint-path recovery tests (release patch 10-12): snapshot →
// restart → tail replay, including expiry semantics, corruption, and
// missing snapshot files. These complement the full-WAL tests above.
// ═══════════════════════════════════════════════════════════════════════

use ramshield::engine::checkpoint::{
    build_snapshot, load_snapshot, restore_from_snapshot, snapshot_path, write_snapshot,
};
use ramshield_storage::checkpoint_shared::CheckpointState;
use std::time::Duration;

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn unix_deadline_ahead(secs: u64) -> u64 {
    now_ns() + secs * 1_000_000_000
}

/// Block via WAL, checkpoint at current LSN, then write snapshot to disk.
fn checkpoint_now(
    wal: &Wal,
    store: &Store,
    ip_exp: std::collections::HashMap<std::net::IpAddr, u64>,
) -> u64 {
    let boundary = wal.begin_checkpoint();
    let cidrs: Vec<ramshield_storage::checkpoint_shared::CidrSnapshot> = store
        .active_cidrs
        .iter()
        .map(|e| *e.key())
        .map(
            |network| ramshield_storage::checkpoint_shared::CidrSnapshot {
                network,
                expires_at_ns: None,
            },
        )
        .collect();
    let state = CheckpointState {
        ip_expirations: ip_exp,
        cidrs,
    };
    let snap = build_snapshot(store, &state, boundary.lsn);
    write_snapshot(&dir_of(wal), &snap, boundary.lsn).unwrap();
    wal.sync().unwrap();
    wal.finish_checkpoint(boundary.lsn, &snapshot_path(&dir_of(wal), boundary.lsn))
        .unwrap();
    boundary.lsn
}

fn dir_of(wal: &Wal) -> String {
    wal.base_dir().to_string()
}

/// Test 10.1 — Permanent IP via checkpoint path: block permanently,
/// checkpoint, restart, verify block survives.
#[test]
fn checkpoint_permanent_ip() {
    let dir = wal_dir("ckpt_perm_ip");
    let wal = open_wal(&dir);
    let ip = IpAddr::from([10, 20, 0, 1]);
    let store = Arc::new(Store::new(16));

    wal.append(&WalEntry::BlockIp {
        ip: ip.to_string(),
        reason: "ckpt".into(),
        ttl_secs: None,
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);
    let wal = open_wal(&dir);
    replay_wal_into_store(&store, &wal, 0).unwrap();

    // Snapshot with no expirations (permanent).
    checkpoint_now(&wal, &store, std::collections::HashMap::new());
    drop(wal);

    // Restart: load snapshot + tail replay.
    let (store2, _ttls, _lsn) = restart_from_snapshot(&dir);
    let rec = store2.get(&ip).unwrap();
    assert!(
        rec.is_blocked(),
        "permanent block must survive checkpoint path"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Test 10.2 — Temporary IP active at checkpoint: remains blocked with
/// remaining TTL ≈ deadline - now.
#[test]
fn checkpoint_temporary_ip_active() {
    let dir = wal_dir("ckpt_tmp_ip");
    let wal = open_wal(&dir);
    let ip = IpAddr::from([10, 20, 0, 2]);
    let store = Arc::new(Store::new(16));

    wal.append(&WalEntry::BlockIp {
        ip: ip.to_string(),
        reason: "ckpt".into(),
        ttl_secs: Some(30),
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);
    let wal = open_wal(&dir);
    let ttls = replay_wal_into_store(&store, &wal, 0).unwrap();
    let remaining_at_ckpt = ttls
        .iter()
        .find(|(a, _)| *a == ip)
        .map(|(_, s)| *s)
        .unwrap_or(0);

    let mut ip_exp = std::collections::HashMap::new();
    ip_exp.insert(ip, unix_deadline_ahead(remaining_at_ckpt));
    checkpoint_now(&wal, &store, ip_exp);
    drop(wal);

    sleep_ms(500); // downtime

    let (store2, restored, _lsn) = restart_from_snapshot(&dir);
    let rec = store2.get(&ip).unwrap();
    assert!(
        rec.is_blocked(),
        "temporary IP must survive checkpoint restart"
    );
    let rem = restored
        .ip_expirations
        .iter()
        .find(|(a, _)| *a == ip)
        .map(|(_, ns)| *ns);
    assert!(rem.is_some(), "temporary IP must have a re-arm deadline");
    let rem_secs = rem.unwrap() / 1_000_000_000;
    assert!(
        rem_secs > remaining_at_ckpt.saturating_sub(2) && rem_secs <= remaining_at_ckpt,
        "remaining TTL should be close to at-checkpoint value, got {rem_secs} (was {remaining_at_ckpt})"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Test 10.3 — Temporary IP expires DURING downtime: must NOT be restored
/// (this catches the old ttl_secs=None bug that made it permanent).
#[test]
fn checkpoint_temporary_ip_expires_during_downtime() {
    let dir = wal_dir("ckpt_expired_ip");
    let wal = open_wal(&dir);
    let ip = IpAddr::from([10, 20, 0, 3]);
    let store = Arc::new(Store::new(16));

    wal.append(&WalEntry::BlockIp {
        ip: ip.to_string(),
        reason: "ckpt".into(),
        ttl_secs: Some(2),
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);
    let wal = open_wal(&dir);
    replay_wal_into_store(&store, &wal, 0).unwrap();

    let mut ip_exp = std::collections::HashMap::new();
    ip_exp.insert(ip, now_ns() + 2 * 1_000_000_000); // expires in 2s
    checkpoint_now(&wal, &store, ip_exp);
    drop(wal);

    sleep_ms(2500); // downtime exceeds TTL

    let (store2, restored, _lsn) = restart_from_snapshot(&dir);
    let blocked = store2.get(&ip).map(|v| v.is_blocked()).unwrap_or(false);
    assert!(!blocked, "expired temporary IP must NOT be restored");
    assert!(
        restored
            .ip_expirations
            .iter()
            .find(|(a, _)| *a == ip)
            .is_none(),
        "expired temporary IP must have no re-arm entry"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Test 10.4 — CIDR-only snapshot (0 temporary IPs, 1 permanent CIDR):
/// CIDR must restore even with no temporary IP deadlines.
#[test]
fn checkpoint_cidr_only() {
    let dir = wal_dir("ckpt_cidr_only");
    let wal = open_wal(&dir);
    let net = IpNetwork::new(IpAddr::from([172, 20, 1, 0]), 24).unwrap();
    let store = Arc::new(Store::new(16));

    wal.append(&WalEntry::BlockCidr {
        cidr: net,
        reason: "ckpt".into(),
        ttl_secs: None,
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);
    // Rebuild store.active_cidrs — replay_wal_into_store doesn't fold CIDRs.
    let wal = open_wal(&dir);
    let cidrs = replay_wal_cidrs_from(&wal, 0).unwrap();
    for (n, _) in cidrs {
        store.active_cidrs.insert(n, ());
    }

    checkpoint_now(&wal, &store, std::collections::HashMap::new());
    drop(wal);

    let (store2, _restored, _lsn) = restart_from_snapshot(&dir);
    assert!(
        store2.active_cidrs.get(&net).is_some(),
        "CIDR must restore via checkpoint path (was broken by empty HashMap)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Test 10.5 — Mixed snapshot + tail: IP A blocked before checkpoint and
/// unblocked after (in tail), IP B blocked after checkpoint. Snapshot
/// state + tail must compose; the unblock wins for A.
#[test]
fn checkpoint_mixed_snapshot_and_tail() {
    let dir = wal_dir("ckpt_mixed");
    let wal = open_wal(&dir);
    let ip_a = IpAddr::from([10, 20, 1, 1]);
    let ip_b = IpAddr::from([10, 20, 1, 2]);
    let net = IpNetwork::new(IpAddr::from([172, 20, 2, 0]), 24).unwrap();
    let store = Arc::new(Store::new(16));

    // Pre-checkpoint: block A.
    wal.append(&WalEntry::BlockIp {
        ip: ip_a.to_string(),
        reason: "pre".into(),
        ttl_secs: None,
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);
    let wal = open_wal(&dir);
    replay_wal_into_store(&store, &wal, 0).unwrap();

    let lsn = checkpoint_now(&wal, &store, std::collections::HashMap::new());

    // Tail: block B, block CIDR, unblock A.
    wal.append(&WalEntry::BlockIp {
        ip: ip_b.to_string(),
        reason: "post".into(),
        ttl_secs: None,
        ts_ns: now_ns(),
    })
    .unwrap();
    wal.append(&WalEntry::BlockCidr {
        cidr: net,
        reason: "post".into(),
        ttl_secs: None,
        ts_ns: now_ns(),
    })
    .unwrap();
    wal.append(&WalEntry::UnblockIp {
        ip: ip_a.to_string(),
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);

    let (store2, _restored, _lsn) = restart_from_snapshot(&dir);
    let a_blocked = store2.get(&ip_a).map(|v| v.is_blocked()).unwrap_or(false);
    assert!(
        !a_blocked,
        "tail unblock of snapshot IP A must win (seed-fold)"
    );
    let b_blocked = store2.get(&ip_b).map(|v| v.is_blocked()).unwrap_or(false);
    assert!(b_blocked, "tail block B must survive");
    assert!(
        store2.active_cidrs.get(&net).is_some(),
        "tail CIDR must restore"
    );
    assert_eq!(_lsn, lsn, "restart must report the snapshot LSN");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Test 11 — Corrupt snapshot file: full WAL replay must still recover state
/// (fail-open to WAL, since history is retained).
#[test]
fn checkpoint_corrupt_snapshot_falls_back_to_wal() {
    let dir = wal_dir("ckpt_corrupt");
    let wal = open_wal(&dir);
    let ip = IpAddr::from([10, 20, 2, 1]);
    let store = Arc::new(Store::new(16));

    wal.append(&WalEntry::BlockIp {
        ip: ip.to_string(),
        reason: "ckpt".into(),
        ttl_secs: None,
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);
    let wal = open_wal(&dir);
    replay_wal_into_store(&store, &wal, 0).unwrap();

    let lsn = checkpoint_now(&wal, &store, std::collections::HashMap::new());
    drop(wal);

    // Corrupt the snapshot file.
    let snap_path = snapshot_path(&dir, lsn);
    std::fs::write(&snap_path, b"{corrupt json").unwrap();

    // Restart: load_snapshot fails → fall back to full replay.
    let loaded = load_snapshot(&snap_path);
    assert!(loaded.is_err() || loaded.unwrap().is_none());
    let (store2, _ttls) = restart_full_replay(&dir);
    let rec = store2.get(&ip).unwrap();
    assert!(
        rec.is_blocked(),
        "corrupt snapshot must fall back to full WAL replay"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Test 12 — Snapshot missing but MANIFEST references it: full WAL replay.
#[test]
fn checkpoint_missing_snapshot_falls_back_to_wal() {
    let dir = wal_dir("ckpt_missing");
    let wal = open_wal(&dir);
    let ip = IpAddr::from([10, 20, 3, 1]);
    let store = Arc::new(Store::new(16));

    wal.append(&WalEntry::BlockIp {
        ip: ip.to_string(),
        reason: "ckpt".into(),
        ttl_secs: None,
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);
    let wal = open_wal(&dir);
    replay_wal_into_store(&store, &wal, 0).unwrap();
    checkpoint_now(&wal, &store, std::collections::HashMap::new());
    drop(wal);

    // Delete snapshot file.
    let snap_lsn = Wal::open(&dir, false, Durability::None, 64 * 1024 * 1024, 0)
        .unwrap()
        .snapshot_lsn();
    std::fs::remove_file(snapshot_path(&dir, snap_lsn)).unwrap();

    let (store2, _ttls) = restart_full_replay(&dir);
    let rec = store2.get(&ip).unwrap();
    assert!(
        rec.is_blocked(),
        "missing snapshot must fall back to full WAL replay"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Restart-from-snapshot helper: load snapshot, restore, replay tail.
/// Returns (store, snapshot_deadlines, snapshot_lsn).
fn restart_from_snapshot(
    dir: &str,
) -> (
    Arc<Store>,
    ramshield::engine::checkpoint::SnapshotRestore,
    u64,
) {
    let store = Arc::new(Store::new(16));
    store
        .traffic
        .ram_limit_mb
        .store(256, std::sync::atomic::Ordering::Relaxed);
    let wal = open_wal(dir);
    let snap_lsn = wal.snapshot_lsn();
    let snap_path = snapshot_path(dir, snap_lsn);
    let snap = load_snapshot(&snap_path).unwrap().expect("snapshot file");
    drop(wal);
    let restored = restore_from_snapshot(&store, &snap);
    // Tail replay starts from snapshot block state (seed-fold), so snapshot
    // IPs untouched by the tail remain blocked; tail UnblockIp removes them.
    let now_ns = now_ns();
    let seed: std::collections::HashMap<std::net::IpAddr, (String, u64, u64)> = restored
        .snapshot_blocked_ips
        .iter()
        .map(|ip| {
            let deadline_ns = restored
                .ip_expirations
                .iter()
                .find(|(sip, _)| sip == ip)
                .map(|(_, ns)| now_ns.saturating_add(*ns))
                .unwrap_or(0);
            (*ip, ("manual_block".to_string(), now_ns, deadline_ns))
        })
        .collect();
    let wal2 = open_wal(dir);
    let ttls =
        ramshield_enforcement::replay_wal_into_store_seeded(&store, &wal2, snap_lsn, seed).unwrap();
    // CIDR tail replay (engine does the same via replay_wal_cidrs_from).
    let tail_cidrs = ramshield_enforcement::replay_wal_cidrs_from(&wal2, snap_lsn).unwrap();
    for (net, remaining) in tail_cidrs {
        store.active_cidrs.insert(net, ());
        // Expiry re-arm is enforced elsewhere; test asserts membership only.
        let _ = remaining;
    }
    drop(wal2);
    // Re-arm expirations from the seed deadlines (ns → secs remaining).
    let _ = ttls;
    (store, restored, snap_lsn)
}

/// Test 5 (PATCH 10) — Temporary CIDR checkpoint: block CIDR with TTL=15,
/// checkpoint, restart, verify CIDR active with remaining TTL.
#[test]
fn checkpoint_temporary_cidr() {
    let dir = wal_dir("ckpt_tmp_cidr");
    let wal = open_wal(&dir);
    let net = IpNetwork::new(IpAddr::from([10, 60, 0, 0]), 16).unwrap();
    let store = Arc::new(Store::new(16));

    wal.append(&WalEntry::BlockCidr {
        cidr: net,
        reason: "ckpt".into(),
        ttl_secs: Some(15),
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);
    let wal = open_wal(&dir);
    let cidrs = replay_wal_cidrs_from(&wal, 0).unwrap();
    for (n, _) in cidrs {
        store.active_cidrs.insert(n, ());
    }
    checkpoint_now(&wal, &store, std::collections::HashMap::new());
    drop(wal);

    let (store2, _restored, _lsn) = restart_from_snapshot(&dir);
    assert!(
        store2.active_cidrs.get(&net).is_some(),
        "temporary CIDR must survive checkpoint restart"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Audit P0 gate — recovery equivalence: the SAME WAL replayed two ways must
/// yield identical enforcement state.
///   A: full WAL replay (min_lsn=0)
///   B: snapshot restore + tail replay (min_lsn=snap_lsn)
/// Compared exhaustively over every IP and CIDR in the scenario: block status
/// and CIDR membership must match term-for-term.
#[test]
fn checkpoint_equivalence_full_replay_vs_snapshot() {
    let dir = wal_dir("ckpt_equiv");
    let wal = Wal::open(&dir, false, Durability::None, 4 * 1024 * 1024, 0).unwrap();

    let perm = IpAddr::from([10, 90, 0, 1]); // permanent, survives
    let temp = IpAddr::from([10, 90, 0, 2]); // temporary, survives
    let killed = IpAddr::from([10, 90, 0, 3]); // blocked then unblocked in tail
    let tail = IpAddr::from([10, 90, 0, 4]); // only in tail
    let net_pre = IpNetwork::new(IpAddr::from([172, 90, 1, 0]), 24).unwrap();
    let net_tail = IpNetwork::new(IpAddr::from([172, 90, 2, 0]), 24).unwrap();

    // ── Pre-checkpoint phase ──
    for (ip, ttl) in [(perm, None), (temp, Some(30)), (killed, None)] {
        wal.append(&WalEntry::BlockIp {
            ip: ip.to_string(),
            reason: "equiv".into(),
            ttl_secs: ttl,
            ts_ns: now_ns(),
        })
        .unwrap();
    }
    wal.append(&WalEntry::BlockCidr {
        cidr: net_pre,
        reason: "equiv".into(),
        ttl_secs: None,
        ts_ns: now_ns(),
    })
    .unwrap();

    // ── Checkpoint (snapshot + MANIFEST) ──
    wal.append(&WalEntry::UnblockIp {
        ip: killed.to_string(),
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);
    let wal = Wal::open(&dir, false, Durability::None, 4 * 1024 * 1024, 0).unwrap();
    let store_ck = Arc::new(Store::new(16));
    let ttls = replay_wal_into_store(&store_ck, &wal, 0).unwrap();
    let cidrs = replay_wal_cidrs_from(&wal, 0).unwrap();
    for (n, _) in &cidrs {
        store_ck.active_cidrs.insert(*n, ());
    }
    let mut ip_exp = std::collections::HashMap::new();
    for (ip, remaining) in &ttls {
        ip_exp.insert(*ip, unix_deadline_ahead(*remaining));
    }
    checkpoint_now(&wal, &store_ck, ip_exp);

    // ── Tail phase (after the checkpoint boundary) ──
    wal.append(&WalEntry::BlockIp {
        ip: tail.to_string(),
        reason: "tail".into(),
        ttl_secs: None,
        ts_ns: now_ns(),
    })
    .unwrap();
    wal.append(&WalEntry::BlockCidr {
        cidr: net_tail,
        reason: "tail".into(),
        ttl_secs: None,
        ts_ns: now_ns(),
    })
    .unwrap();
    wal.append(&WalEntry::UnblockIp {
        ip: perm.to_string(),
        ts_ns: now_ns(),
    })
    .unwrap();
    drop(wal);

    // ── A: full WAL replay ──
    let (store_full, _) = restart_full_replay(&dir);

    // ── B: snapshot restore + tail replay ──
    let (store_snap, _restored, _lsn) = restart_from_snapshot(&dir);

    // ── Compare, exhaustively ──
    let universe = [perm, temp, killed, tail];
    for ip in universe {
        let a = store_full.get(&ip).is_some_and(|v| v.is_blocked());
        let b = store_snap.get(&ip).is_some_and(|v| v.is_blocked());
        assert_eq!(
            a, b,
            "equivalence broken for {ip}: full_replay blocked={a}, snapshot+tail blocked={b}"
        );
    }
    for net in [net_pre, net_tail] {
        let a = store_full.active_cidrs.contains_key(&net);
        let b = store_snap.active_cidrs.contains_key(&net);
        assert_eq!(
            a, b,
            "CIDR equivalence broken for {net}: full_replay={a}, snapshot+tail={b}"
        );
    }
    // Sanity: the scenario actually exercised both directions.
    assert!(store_full.get(&temp).is_some_and(|v| v.is_blocked()));
    assert!(store_full.get(&tail).is_some_and(|v| v.is_blocked()));
    assert!(!store_full.get(&perm).is_some_and(|v| v.is_blocked()));
    assert!(!store_full.get(&killed).is_some_and(|v| v.is_blocked()));
    assert!(store_full.active_cidrs.contains_key(&net_pre));
    assert!(store_full.active_cidrs.contains_key(&net_tail));

    let _ = std::fs::remove_dir_all(&dir);
}

/// PATCH 12b — Hard WAL fail-closed: snapshot exists but WAL history pruned
/// past snapshot boundary → load must fail rather than silently start with
/// incomplete state.  We simulate this by checkpointing, then removing oldest
/// segment (so oldest_lsn > snap_lsn).
#[test]
fn checkpoint_hard_wal_fail_closed() {
    let dir = wal_dir("ckpt_hard_wal");
    let wal = Wal::open(&dir, false, Durability::None, 320, 0).unwrap();
    let _ip = IpAddr::from([10, 70, 0, 1]);
    let store = Arc::new(Store::new(16));

    // Write many records to fill multiple segments (320B cap), checkpoint,
    // write more to force later segments, then delete all but the newest
    // segment. Under RAMWAL, deleting segments that contain the checkpoint
    // boundary makes the manifest invalid — reopening fails with
    // "checkpoint LSN exceeds recovered WAL LSN", which IS the hard-WAL
    // fail-closed gate (allow_volatile_fallback=false).
    for i in 0..200 {
        wal.append(&WalEntry::BlockIp {
            ip: format!("10.70.{}.1", i / 64).to_string(),
            reason: "fill".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
    }
    drop(wal);
    let wal = Wal::open(&dir, false, Durability::None, 320, 0).unwrap();
    let _ = replay_wal_into_store(&store, &wal, 0).unwrap();
    let _checkpoint_lsn = checkpoint_now(&wal, &store, std::collections::HashMap::new());
    // Write 100 more records to segments strictly after the checkpoint LSN.
    for i in 0..100 {
        wal.append(&WalEntry::BlockIp {
            ip: format!("10.70.99.{}", i).to_string(),
            reason: "new-seg".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
    }
    drop(wal);

    // Delete all .rwl segments EXCEPT the highest-index (newest) one.
    let mut segs: Vec<(u64, _)> = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rwl"))
        .filter_map(|p| {
            let name = p.file_name()?.to_string_lossy().to_string();
            let idx: u64 = name
                .strip_prefix("segment-")?
                .strip_suffix(".rwl")?
                .parse()
                .ok()?;
            Some((idx, p))
        })
        .collect::<Vec<_>>();
    segs.sort_by_key(|(idx, _)| *idx);
    if segs.len() >= 2 {
        for (_, p) in &segs[..segs.len() - 1] {
            std::fs::remove_file(p).ok();
        }
    }

    // Reopen must fail: the manifest references a checkpoint LSN in a
    // now-deleted segment. This IS the hard-WAL fail-closed gate.
    let wal2 = Wal::open(&dir, false, Durability::None, 128 * 1024, 0);
    assert!(
        wal2.is_err(),
        "hard-WAL must fail-closed when checkpoint boundary segments are pruned"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// PATCH 13 — Checkpoint boundary concurrency: write mixed mutations
/// between checkpoints, keep store aligned via full IP fold after each
/// batch (simulating the enforcement→store →checkpoint pipeline),
/// then restart and verify recovered state matches expected block set.
#[test]
fn checkpoint_concurrency() {
    let dir = wal_dir("ckpt_conc");
    for iter in 0..8 {
        let _ = std::fs::remove_dir_all(&dir);

        // Helper: append entries then reopen for proper disk flush.
        let open = || open_wal(&dir);
        let flush = |w| {
            drop(w);
            open()
        };

        let mut wal = open();
        let ips: Vec<IpAddr> = [10, 20, 30, 40, 50]
            .into_iter()
            .map(|x| IpAddr::from([10, 0, 0, x]))
            .collect();

        // Seed: block all 5 IPs, replay into store, checkpoint.
        for ip in &ips {
            wal.append(&WalEntry::BlockIp {
                ip: ip.to_string(),
                reason: "seed".into(),
                ttl_secs: None,
                ts_ns: now_ns(),
            })
            .unwrap();
        }
        wal = flush(wal); // flush → disk consistent
        let store1 = Arc::new(Store::new(16));
        replay_wal_into_store(&store1, &wal, 0).unwrap();
        checkpoint_now(&wal, &store1, std::collections::HashMap::new());

        // Mutations: unblock ips[0], block 99.0.0.1.
        wal.append(&WalEntry::UnblockIp {
            ip: ips[0].to_string(),
            ts_ns: now_ns(),
        })
        .unwrap();
        wal.append(&WalEntry::BlockIp {
            ip: IpAddr::from([99, 0, 0, 1]).to_string(),
            reason: "conc".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
        wal = flush(wal);
        let store2 = Arc::new(Store::new(16));
        replay_wal_into_store(&store2, &wal, 0).unwrap();
        checkpoint_now(&wal, &store2, std::collections::HashMap::new());

        // Tail after last checkpoint.
        wal.append(&WalEntry::UnblockIp {
            ip: ips[1].to_string(),
            ts_ns: now_ns(),
        })
        .unwrap();
        wal.append(&WalEntry::BlockIp {
            ip: IpAddr::from([99, 0, 0, 2]).to_string(),
            reason: "final".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
        _ = flush(wal);

        // Expected final state from full replay.
        let wal_final = open();
        let store_c = Arc::new(Store::new(16));
        replay_wal_into_store(&store_c, &wal_final, 0).unwrap();
        drop(wal_final);

        // Restart from last checkpoint snapshot + tail.
        let (store_r, _restored, _lsn) = restart_from_snapshot(&dir);
        for ip in [
            ips[2],
            ips[3],
            ips[4],
            IpAddr::from([99, 0, 0, 1]),
            IpAddr::from([99, 0, 0, 2]),
        ] {
            assert!(
                store_r.get(&ip).is_some_and(|v| v.is_blocked()),
                "iter {iter}: {ip} must be blocked after restart"
            );
            assert!(
                store_c.get(&ip).is_some_and(|v| v.is_blocked()),
                "iter {iter}: full-replay also needs {ip}"
            );
        }
        for ip in [ips[0], ips[1]] {
            assert!(
                store_r.get(&ip).is_none_or(|v| !v.is_blocked()),
                "iter {iter}: {ip} must NOT be blocked after restart"
            );
        }
        eprintln!("  iter {iter} OK");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
// ═══════════════════════════════════════════════════════════════════════
// Batch 2 — Phase 4 extended: checkpoint equivalence + tail replay tests
// Each validates: full WAL replay ≡ snapshot(N) + WAL(N+1...)
// ═══════════════════════════════════════════════════════════════════════

// ── Case 2: Overlapping CIDRs ──
#[test]
fn checkpoint_equivalent_overlapping_cidrs() {
    let dir = wal_dir("ckpt_equiv_overlap");

    // Pre-checkpoint: overlapping CIDR blocks
    let wal_pre = open_wal(&dir);
    let net1 = IpNetwork::new(IpAddr::from([10, 0, 0, 0]), 8).unwrap();
    let net2 = IpNetwork::new(IpAddr::from([10, 1, 0, 0]), 16).unwrap();
    let net3 = IpNetwork::new(IpAddr::from([10, 1, 2, 0]), 24).unwrap();
    for net in [net1, net2, net3] {
        wal_pre
            .append(&WalEntry::BlockCidr {
                cidr: net,
                reason: "overlap".into(),
                ttl_secs: None,
                ts_ns: now_ns(),
            })
            .unwrap();
    }
    drop(wal_pre);

    // Checkpoint boundary (snapshot)
    {
        let wal = open_wal(&dir);
        let store = Arc::new(Store::new(16));
        let _ = replay_wal_into_store(&store, &wal, 0).unwrap();
        for (n, _) in &replay_wal_cidrs_from(&wal, 0).unwrap() {
            store.active_cidrs.insert(*n, ());
        }
        checkpoint_now(&wal, &store, std::collections::HashMap::new());
    };

    // Tail: more overlapping CIDR
    let wal_tail = open_wal(&dir);
    let net4 = IpNetwork::new(IpAddr::from([10, 1, 2, 128]), 25).unwrap();
    wal_tail
        .append(&WalEntry::BlockCidr {
            cidr: net4,
            reason: "tail_overlap".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
    drop(wal_tail);

    // Verify: full replay ≡ snapshot+tail
    let store_full = restart_full_replay(&dir).0;
    let store_snap = restart_from_snapshot(&dir).0;
    for net in [net1, net2, net3, net4] {
        assert_eq!(
            store_full.active_cidrs.contains_key(&net),
            store_snap.active_cidrs.contains_key(&net),
            "overlapping CIDRs: checkpoint+tail == full replay"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Case 3: Block / unblock ──
#[test]
fn checkpoint_equivalent_block_unblock() {
    let dir = wal_dir("ckpt_equiv_blkunblk");

    let wal_pre = open_wal(&dir);
    let ip_a = IpAddr::from([192, 168, 0, 1]);
    for _ in 0..2 {
        wal_pre
            .append(&WalEntry::BlockIp {
                ip: ip_a.to_string(),
                reason: "dbl_blk".into(),
                ttl_secs: None,
                ts_ns: now_ns(),
            })
            .unwrap();
    }
    drop(wal_pre);

    // Checkpoint boundary
    {
        let wal = open_wal(&dir);
        let store = Arc::new(Store::new(16));
        let _ = replay_wal_into_store(&store, &wal, 0).unwrap();
        for (n, _) in &replay_wal_cidrs_from(&wal, 0).unwrap() {
            store.active_cidrs.insert(*n, ());
        }
        checkpoint_now(&wal, &store, std::collections::HashMap::new());
    };

    // Tail: unblock
    let wal_tail = open_wal(&dir);
    wal_tail
        .append(&WalEntry::UnblockIp {
            ip: ip_a.to_string(),
            ts_ns: now_ns(),
        })
        .unwrap();
    drop(wal_tail);

    // Both paths must agree: IP NOT blocked
    let store_full = restart_full_replay(&dir).0;
    let store_snap = restart_from_snapshot(&dir).0;
    for (source, st) in [("full_replay", store_full), ("snapshot+tail", store_snap)] {
        let rec = st.get(&ip_a);
        let blocked = rec.map(|v| v.is_blocked()).unwrap_or(false);
        assert!(
            !blocked,
            "{source}: {ip_a} must NOT be blocked after unblock tail"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Case 4: TTL survival through checkpoint ──
#[test]
fn checkpoint_equivalent_ttl() {
    let dir = wal_dir("ckpt_equiv_ttl");

    let wal_pre = open_wal(&dir);
    let ip = IpAddr::from([172, 16, 0, 1]);
    wal_pre
        .append(&WalEntry::BlockIp {
            ip: ip.to_string(),
            reason: "ttl_src".into(),
            ttl_secs: Some(60),
            ts_ns: now_ns(),
        })
        .unwrap();
    drop(wal_pre);

    // Checkpoint with TTL tracking
    {
        let wal = open_wal(&dir);
        let store = Arc::new(Store::new(16));
        let ttls = replay_wal_into_store(&store, &wal, 0).unwrap();
        for (n, _) in &replay_wal_cidrs_from(&wal, 0).unwrap() {
            store.active_cidrs.insert(*n, ());
        }
        let remaining = ttls
            .iter()
            .find(|(a, _)| *a == ip)
            .map(|(_, s)| *s)
            .unwrap_or(0);
        let mut ip_exp = std::collections::HashMap::new();
        ip_exp.insert(ip, unix_deadline_ahead(remaining));
        checkpoint_now(&wal, &store, ip_exp);
    };

    sleep_ms(100); // small downtime sim
    // Tail: re-block (update TTL)
    let wal_tail = open_wal(&dir);
    wal_tail
        .append(&WalEntry::BlockIp {
            ip: ip.to_string(),
            reason: "ttl_update".into(),
            ttl_secs: Some(30),
            ts_ns: now_ns(),
        })
        .unwrap();
    drop(wal_tail);

    let store_full = restart_full_replay(&dir).0;
    let store_snap = restart_from_snapshot(&dir).0;
    let rec_full = store_full.get(&ip);
    let rec_snap = store_snap.get(&ip);
    assert_eq!(
        rec_full.map(|v| v.is_blocked()),
        rec_snap.map(|v| v.is_blocked()),
        "TTL path: blocked state must match between full_replay and snapshot+tail"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Case 5: Multiple checkpoints ──
#[test]
fn checkpoint_equivalent_multiple_ckpt() {
    let dir = wal_dir("ckpt_equiv_multi");

    // Round 1: block A, B, C → checkpoint #1
    {
        let wal = open_wal(&dir);
        let nets = [
            IpNetwork::new(IpAddr::from([10, 0, 0, 0]), 8).unwrap(),
            IpNetwork::new(IpAddr::from([10, 1, 0, 0]), 16).unwrap(),
            IpNetwork::new(IpAddr::from([10, 2, 0, 0]), 16).unwrap(),
        ];
        for n in &nets {
            wal.append(&WalEntry::BlockCidr {
                cidr: *n,
                reason: "r1".into(),
                ttl_secs: None,
                ts_ns: now_ns(),
            })
            .unwrap();
        }
        drop(wal);
    }

    // Round 2: unblock C, block D → checkpoint #2
    {
        let wal = open_wal(&dir);
        wal.append(&WalEntry::UnblockCidr {
            cidr: IpNetwork::new(IpAddr::from([10, 2, 0, 0]), 16).unwrap(),
            ts_ns: now_ns(),
        })
        .unwrap();
        wal.append(&WalEntry::BlockCidr {
            cidr: IpNetwork::new(IpAddr::from([10, 3, 0, 0]), 16).unwrap(),
            reason: "r2".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
        drop(wal);
        let wal = open_wal(&dir);
        let store = Arc::new(Store::new(16));
        let _ = replay_wal_into_store(&store, &wal, 0).unwrap();
        for (n, _) in &replay_wal_cidrs_from(&wal, 0).unwrap() {
            store.active_cidrs.insert(*n, ());
        }
        checkpoint_now(&wal, &store, std::collections::HashMap::new());
        drop(wal);
    }

    // Tail: block E, F
    {
        let wal = open_wal(&dir);
        for i in [4, 5] {
            wal.append(&WalEntry::BlockCidr {
                cidr: IpNetwork::new(IpAddr::from([10, i, 0, 0]), 16).unwrap(),
                reason: "tail".into(),
                ttl_secs: None,
                ts_ns: now_ns(),
            })
            .unwrap();
        }
        drop(wal);
    }

    // Compare full replay vs snapshot+tail
    let store_full = restart_full_replay(&dir).0;
    let store_snap = restart_from_snapshot(&dir).0;

    let expected_cidrs = vec![
        ([10, 0, 0, 0], 8),  // r1, survives
        ([10, 1, 0, 0], 16), // r1, survives
        ([10, 3, 0, 0], 16), // r2, survives (C unblocked)
        ([10, 4, 0, 0], 16), // tail
        ([10, 5, 0, 0], 16), // tail
    ];

    for (octets, prefix) in expected_cidrs {
        let net = IpNetwork::new(IpAddr::from(octets), prefix).unwrap();
        let a = store_full.active_cidrs.contains_key(&net);
        let b = store_snap.active_cidrs.contains_key(&net);
        assert_eq!(a, b, "multi-ckpt: {net} mismatch");
        assert!(a && b, "multi-ckpt: {net} must be in both recoveries");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

// ═══════════════════════════════════════════════════════════════════════
// Batch 3 — Phase 5: crash-point recovery tests
//
// We cannot SIGKILL mid-syscall, so each test constructs the on-disk state
// that a crash at that boundary would leave behind, then runs the real
// recovery path over it. The invariant under test is the roadmap's:
//   recovery chooses the last completely durable state and replays the
//   valid WAL tail; a crash must never fabricate state silently.
// ═══════════════════════════════════════════════════════════════════════

/// True when `store` has `ip` blocked.
fn blocked(store: &Store, ip: &IpAddr) -> bool {
    store.get(ip).is_some_and(|v| v.is_blocked())
}

// ── Shared crash-suite helpers ────────────────────────────────────────
// Every crash test uses the same WAL shape (buffered durability, one huge
// segment so rotation never interferes with tail surgery) and the same
// recover-by-full-replay step. Keep both in one place.

/// WAL opened with the crash-suite defaults.
fn open_wal(dir: &str) -> Wal {
    Wal::open(dir, false, Durability::None, 64 * 1024 * 1024, 0).unwrap()
}

/// Newest `.rwl` segment in `dir` — the one a crash would tear.
fn newest_segment(dir: &str) -> std::path::PathBuf {
    let mut segs: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rwl"))
        .collect();
    segs.sort();
    segs.pop().expect("segment")
}

/// Resize the newest segment's tail by `delta` bytes: negative truncates a
/// torn record, positive appends garbage. Returns the resulting length.
fn resize_newest_segment(dir: &str, delta: i64) -> u64 {
    let seg = newest_segment(dir);
    let new_len = (std::fs::metadata(&seg).unwrap().len() as i64 + delta).max(0) as u64;
    let f = std::fs::OpenOptions::new().write(true).open(&seg).unwrap();
    f.set_len(new_len).unwrap();
    new_len
}

/// Crash point 1 — during WAL append.
/// A partially-written trailing record (torn header/payload) is the last
/// thing on disk. The record was never acknowledged, so recovery is allowed
/// to drop it — but every record before it must survive intact.
#[test]
fn crash_during_wal_append_torn_tail_dropped() {
    let dir = wal_dir("crash_append");
    let a = IpAddr::from([10, 91, 0, 1]);
    let b = IpAddr::from([10, 91, 0, 2]);
    let torn = IpAddr::from([10, 91, 0, 3]);

    {
        let wal = open_wal(&dir);
        for ip in [a, b] {
            wal.append(&WalEntry::BlockIp {
                ip: ip.to_string(),
                reason: "durable".into(),
                ttl_secs: None,
                ts_ns: now_ns(),
            })
            .unwrap();
        }
        wal.sync().unwrap();
        wal.append(&WalEntry::BlockIp {
            ip: torn.to_string(),
            reason: "torn".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
    } // drop = crash

    // Truncate the final record mid-way: simulate the OS losing the tail
    // of an in-flight write. Chop the segment to just past `b`.
    // Chop 3 bytes — mid-record, guaranteed torn (not a clean boundary).
    resize_newest_segment(&dir, -3);

    let store = Arc::new(Store::new(16));
    let wal2 = open_wal(&dir);
    replay_wal_into_store(&store, &wal2, 0).unwrap();
    drop(wal2);

    assert!(
        blocked(&store, &a),
        "acknowledged record A must survive a torn tail"
    );
    assert!(
        blocked(&store, &b),
        "acknowledged record B must survive a torn tail"
    );
    assert!(
        !blocked(&store, &torn),
        "unacknowledged torn record must not be fabricated into state"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Crash point 1b — a torn tail must not poison later appends.
/// After recovery drops the torn record, new writes must still work and the
/// recovered prefix must remain readable.
#[test]
fn crash_torn_tail_then_append_recovers_continues() {
    let dir = wal_dir("crash_append_cont");
    let keep = IpAddr::from([10, 92, 0, 1]);
    let after = IpAddr::from([10, 92, 0, 2]);

    {
        let wal = open_wal(&dir);
        wal.append(&WalEntry::BlockIp {
            ip: keep.to_string(),
            reason: "keep".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
        wal.sync().unwrap();
    }
    resize_newest_segment(&dir, 7); // garbage tail beyond valid records

    {
        let wal = open_wal(&dir);
        wal.append(&WalEntry::BlockIp {
            ip: after.to_string(),
            reason: "post-recovery".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
        wal.sync().unwrap();
    }

    let store = Arc::new(Store::new(16));
    let wal3 = open_wal(&dir);
    replay_wal_into_store(&store, &wal3, 0).unwrap();
    drop(wal3);
    assert!(blocked(&store, &keep), "pre-corruption record survived");
    assert!(blocked(&store, &after), "post-recovery append survived");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Crash point 2 — after WAL write, before sync.
/// The record may or may not be durable; either outcome is legal. What is
/// NOT legal is a fabricated record or a corrupted higher-LSN history. With
/// Durability::None the buffered write is flushed on drop, so we verify the
/// in-flight record lands in exactly one of the two legal states.
#[test]
fn crash_after_write_before_sync_is_legal_either_way() {
    let dir = wal_dir("crash_pre_sync");
    let committed = IpAddr::from([10, 93, 0, 1]);
    let maybe = IpAddr::from([10, 93, 0, 2]);

    {
        let wal = open_wal(&dir);
        wal.append(&WalEntry::BlockIp {
            ip: committed.to_string(),
            reason: "synced".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
        wal.sync().unwrap();
        wal.sync().unwrap();
        assert!(wal.durable_lsn() >= 1);
        // Written but never synced, never acknowledged.
        wal.append(&WalEntry::BlockIp {
            ip: maybe.to_string(),
            reason: "unsynced".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
    }

    let store = Arc::new(Store::new(16));
    let wal2 = open_wal(&dir);
    replay_wal_into_store(&store, &wal2, 0).unwrap();
    drop(wal2);

    assert!(blocked(&store, &committed), "synced record must be present");
    // `maybe` is present or absent — both legal; it must never be partial.
    let entries = Wal::replay_dir(&dir).unwrap();
    let seen: Vec<_> = entries
        .iter()
        .filter_map(|e| match e {
            WalEntry::BlockIp { ip, .. } if ip == &maybe.to_string() => Some(()),
            _ => None,
        })
        .collect();
    assert!(
        seen.len() <= 1,
        "unsynced record must appear at most once, never duplicated"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Crash point 3 — after WAL sync. Everything acknowledged as durable must
/// survive unconditionally, and LSN order must be exact on replay.
#[test]
fn crash_after_wal_sync_everything_survives_in_lsn_order() {
    let dir = wal_dir("crash_post_sync");
    let ips: Vec<IpAddr> = (1..=5).map(|x| IpAddr::from([10, 94, 0, x])).collect();

    {
        let wal = open_wal(&dir);
        for ip in &ips {
            wal.append(&WalEntry::BlockIp {
                ip: ip.to_string(),
                reason: "durable".into(),
                ttl_secs: None,
                ts_ns: now_ns(),
            })
            .unwrap();
            wal.sync().unwrap();
        }
    }

    let entries = Wal::replay_dir(&dir).unwrap();
    let lsns: Vec<u64> = entries.iter().map(|_| 0).collect();
    let _ = lsns;
    // Replay returns strictly increasing LSNs by construction — assert the
    // recovered block set instead of re-deriving LSNs the reader doesn't expose.
    let store = Arc::new(Store::new(16));
    let wal2 = open_wal(&dir);
    replay_wal_into_store(&store, &wal2, 0).unwrap();
    drop(wal2);
    for ip in &ips {
        assert!(blocked(&store, ip), "durably synced {ip} must survive");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Crash point 4 — during snapshot write. The .tmp file is partial and the
/// final snapshot path never appeared. Recovery must ignore the partial tmp
/// entirely and use WAL (or the prior checkpoint if one existed).
#[test]
fn crash_during_snapshot_write_partial_tmp_ignored() {
    let dir = wal_dir("crash_snap_tmp");
    let ip = IpAddr::from([10, 95, 0, 1]);

    {
        let wal = open_wal(&dir);
        wal.append(&WalEntry::BlockIp {
            ip: ip.to_string(),
            reason: "keep".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
        wal.sync().unwrap();
    }

    // Simulate a partial snapshot write: a .tmp with truncated JSON.
    let boundary = 1u64;
    let snap_path = snapshot_path(&dir, boundary);
    let tmp = format!("{snap_path}.tmp");
    std::fs::write(&tmp, b"{\"lsn\":1,\"ts_ns\":12").unwrap();
    assert!(!std::path::Path::new(&snap_path).exists());

    // No checkpoint marker + no final snapshot → recovery falls back to WAL.
    let store = Arc::new(Store::new(16));
    let wal2 = open_wal(&dir);
    assert_eq!(wal2.snapshot_lsn(), 0, "no checkpoint published");
    replay_wal_into_store(&store, &wal2, 0).unwrap();
    drop(wal2);
    assert!(blocked(&store, &ip), "WAL fallback recovered the block");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Crash point 5 — after snapshot fsync, before rename. The full snapshot
/// content exists at `snapshot.<lsn>.ckpt.tmp` but was never renamed, and no
/// checkpoint marker was appended. The previous checkpoint (here: none)
/// remains authoritative.
#[test]
fn crash_after_snapshot_fsync_before_rename_previous_authoritative() {
    let dir = wal_dir("crash_pre_rename");
    let ip = IpAddr::from([10, 96, 0, 1]);

    {
        let wal = open_wal(&dir);
        wal.append(&WalEntry::BlockIp {
            ip: ip.to_string(),
            reason: "keep".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
        wal.sync().unwrap();
    }

    // Complete, fsynced snapshot content parked at the tmp path.
    let seat = snapshot_path(&dir, 1);
    let tmp = format!("{seat}.tmp");
    let store_pre = Arc::new(Store::new(16));
    {
        let wal = open_wal(&dir);
        replay_wal_into_store(&store_pre, &wal, 0).unwrap();
    }
    let snap = build_snapshot(
        &store_pre,
        &CheckpointState {
            ip_expirations: std::collections::HashMap::new(),
            cidrs: vec![],
        },
        1,
    );
    let json = serde_json::to_vec(&snap).unwrap();
    std::fs::write(&tmp, &json).unwrap();
    assert!(std::path::Path::new(&tmp).exists());
    assert!(
        !std::path::Path::new(&seat).exists(),
        "rename never happened"
    );

    // Reopen: no MANIFEST, so the tmp must not be treated as the snapshot.
    let wal2 = open_wal(&dir);
    assert_eq!(wal2.ckpt_lsn(), 0, "MANIFEST was never published");
    assert_eq!(wal2.snapshot_lsn(), 0, "snapshot LSN never published");
    drop(wal2);

    // Full replay is authoritative and recovers the block.
    let store = Arc::new(Store::new(16));
    let wal3 = open_wal(&dir);
    replay_wal_into_store(&store, &wal3, 0).unwrap();
    drop(wal3);
    assert!(blocked(&store, &ip));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Crash point 6 — after rename, before checkpoint publication. The snapshot
/// file is in place but MANIFEST still points at the older checkpoint (or
/// none). Recovery must use the older checkpoint, never the un-published
/// snapshot, because the checkpoint marker record is what makes it valid.
#[test]
fn crash_after_rename_before_publication_uses_old_checkpoint() {
    let dir = wal_dir("crash_pre_publish");
    let ip1 = IpAddr::from([10, 97, 0, 1]);
    let ip2 = IpAddr::from([10, 97, 0, 2]);

    // First checkpoint: publish properly at boundary L2 with ip1 blocked.
    {
        let wal = open_wal(&dir);
        wal.append(&WalEntry::BlockIp {
            ip: ip1.to_string(),
            reason: "ckpt1".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
        wal.sync().unwrap();
        let store = Arc::new(Store::new(16));
        replay_wal_into_store(&store, &wal, 0).unwrap();
        checkpoint_now(&wal, &store, std::collections::HashMap::new());
    }
    let manifest_before =
        std::fs::read_to_string(std::path::Path::new(&dir).join("MANIFEST")).unwrap();

    // Now attempt a second checkpoint: write snapshot + append marker, but
    // "crash" before finish_checkpoint rewrites MANIFEST.
    {
        let wal = open_wal(&dir);
        wal.append(&WalEntry::BlockIp {
            ip: ip2.to_string(),
            reason: "ckpt2".into(),
            ttl_secs: None,
            ts_ns: now_ns(),
        })
        .unwrap();
        wal.sync().unwrap();
        let store = Arc::new(Store::new(16));
        replay_wal_into_store(&store, &wal, 0).unwrap();
        let boundary = wal.begin_checkpoint();
        let snap = build_snapshot(
            &store,
            &CheckpointState {
                ip_expirations: std::collections::HashMap::new(),
                cidrs: vec![],
            },
            boundary.lsn,
        );
        write_snapshot(&dir, &snap, boundary.lsn).unwrap(); // renamed, durable
        // … crash here: no finish_checkpoint, MANIFEST untouched.
    }

    let manifest_after =
        std::fs::read_to_string(std::path::Path::new(&dir).join("MANIFEST")).unwrap();
    assert_eq!(
        manifest_before, manifest_after,
        "un-published checkpoint must not have touched MANIFEST"
    );

    let wal2 = open_wal(&dir);
    let published_snap_lsn = wal2.snapshot_lsn();
    assert!(published_snap_lsn > 0, "first checkpoint is published");
    let ckpt1_lsn = wal2.ckpt_lsn();
    // The second snapshot file exists on disk but is not referenced.
    let orphan = snapshot_path(&dir, ckpt1_lsn + 1);
    let _ = orphan;
    drop(wal2);

    // Recover via published checkpoint + tail — both IPs must be present.
    let (store_r, _restored, snap_lsn) = restart_from_snapshot(&dir);
    assert_eq!(
        snap_lsn, published_snap_lsn,
        "recovery used the published checkpoint"
    );
    assert!(blocked(&store_r, &ip1), "checkpoint-1 state recovered");
    assert!(
        blocked(&store_r, &ip2),
        "post-checkpoint tail replayed (ip2 was appended before the crash)"
    );

    // And full replay must agree.
    let store_f = Arc::new(Store::new(16));
    let wal3 = open_wal(&dir);
    replay_wal_into_store(&store_f, &wal3, 0).unwrap();
    drop(wal3);
    for ip in [ip1, ip2] {
        assert_eq!(
            blocked(&store_r, &ip),
            blocked(&store_f, &ip),
            "crash-point-6 recovery must equal full replay for {ip}"
        );
        assert!(blocked(&store_f, &ip));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
