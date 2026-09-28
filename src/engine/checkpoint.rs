//! Checkpoint snapshot: durable image of enforcement state at an LSN.
//! Written by the engine on a periodic loop; loaded at boot to skip
//! replaying historical WAL entries before the checkpoint boundary.
//!
//! Expirations are persisted as ABSOLUTE Unix-epoch ns deadlines, not
//! remaining seconds — a restored checkpoint's remaining TTL is computed
//! at recovery time against the recovery clock, so downtime shrinks (and
//! can expire) temporary blocks exactly like live operation would.

use ramshield_storage::{
    BlockState as StorageBlockState, IpRecord as StorageIpRecord, Store, Value,
    checkpoint_shared::{CheckpointState, CidrSnapshot},
};
use ramshield_types::{BlockReason, IpNetwork, Result, RsError};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::net::IpAddr;
use std::path::PathBuf;

/// Per-IP block state carried in a checkpoint.
#[derive(Debug, Serialize, Deserialize)]
pub struct IpBlockSnapshot {
    pub ip: IpAddr,
    pub reason: String,
    pub since_ns: u64,
    /// None → permanent block. Some(ns) → absolute Unix-epoch deadline ns.
    pub expires_at_ns: Option<u64>,
}

/// Snapshot of enforcement state at a given LSN.
/// On recovery: load snapshot, then replay WAL entries > lsn.
#[derive(Debug, Serialize, Deserialize)]
pub struct CheckpointSnapshot {
    pub lsn: u64,
    pub ts_ns: u64,
    pub blocked_ips: Vec<IpBlockSnapshot>,
    pub active_cidrs: Vec<CidrSnapshot>,
}

/// Build snapshot path from WAL base dir and LSN.
pub fn snapshot_path(wal_dir: &str, lsn: u64) -> String {
    PathBuf::from(wal_dir)
        .join(format!("snapshot.{lsn:020}.ckpt"))
        .to_string_lossy()
        .to_string()
}

/// Write snapshot atomically: tmp + fsync + rename.
pub fn write_snapshot(dir: &str, snap: &CheckpointSnapshot, lsn: u64) -> Result<String> {
    let path = snapshot_path(dir, lsn);
    let tmp = format!("{path}.tmp");
    let json = serde_json::to_vec(snap).map_err(|e| RsError::Serde(e.to_string()))?;
    {
        let mut f = File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    fsync_dir(dir)?;
    Ok(path)
}

/// Load snapshot from file. Returns Ok(None) when file does not exist.
/// Corrupt content returns an error — the caller decides whether to fall
/// back to full WAL replay.
pub fn load_snapshot(path: &str) -> Result<Option<CheckpointSnapshot>> {
    let p = PathBuf::from(path);
    if !p.exists() {
        return Ok(None);
    }
    let mut f = File::open(path)?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes)?;
    let snap = serde_json::from_slice::<CheckpointSnapshot>(&bytes)
        .map_err(|e| RsError::Serde(e.to_string()))?;
    Ok(Some(snap))
}

pub fn snapshot_exists(wal_dir: &str, lsn: u64) -> bool {
    PathBuf::from(snapshot_path(wal_dir, lsn)).exists()
}

fn fsync_dir(dir: &str) -> Result<()> {
    let d = File::open(dir)?;
    d.sync_all()?;
    Ok(())
}

pub fn now_unix_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Build a snapshot from the live store + explicit checkpoint state.
///
/// `state` carries the enforcement-owned absolute deadlines (IP + CIDR);
/// the snapshot MUST represent actual enforcement state, never a guessed
/// TTL. Permanent blocks carry `expires_at_ns: None`; temporary ones carry
/// the live deadline from the mirror.
pub fn build_snapshot(store: &Store, state: &CheckpointState, lsn: u64) -> CheckpointSnapshot {
    let now_ns = now_unix_ns();
    let blocked_ips: Vec<IpBlockSnapshot> = store
        .get_all_blocked_ips()
        .into_iter()
        .filter_map(|ip| {
            let value = store.get(&ip)?;
            let Value::IpRecord(record) = value else {
                return None;
            };
            let StorageBlockState::Blocked {
                ref reason,
                since_ns,
            } = record.block_state
            else {
                return None;
            };
            Some(IpBlockSnapshot {
                ip,
                reason: reason.as_str().to_string(),
                since_ns,
                expires_at_ns: state.ip_expirations.get(&ip).copied(),
            })
        })
        .collect();
    let active_cidrs: Vec<CidrSnapshot> = state
        .cidrs
        .iter()
        .map(|c| CidrSnapshot {
            network: c.network,
            expires_at_ns: c.expires_at_ns,
        })
        .collect();
    CheckpointSnapshot {
        lsn,
        ts_ns: now_ns,
        blocked_ips,
        active_cidrs,
    }
}

/// Expired-snapshot-block policy for `restore_from_snapshot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpiryVerdict {
    /// Block is live; carry the remaining deadline (ns).
    Live { remaining_ns: u64 },
    /// Deadline already passed (or clock went backwards past it).
    Expired,
    /// No deadline — permanent.
    Permanent,
}

fn classify(expires_at_ns: Option<u64>, now_ns: u64) -> ExpiryVerdict {
    match expires_at_ns {
        None => ExpiryVerdict::Permanent,
        Some(deadline) if deadline > now_ns => ExpiryVerdict::Live {
            remaining_ns: deadline - now_ns,
        },
        Some(_) => ExpiryVerdict::Expired,
    }
}

/// Restore store block records from a snapshot.
///
/// IP records are inserted as Blocked (permanent for `None` deadlines;
/// temporary ones re-inserted with their remaining TTL so Store's passive
/// expiry wheel matches enforcement's schedule). CIDR membership is written
/// into `store.active_cidrs` — the single userspace authority.
///
/// Returns `SnapshotRestoreState`: the absolute deadlines that enforcement
/// must re-arm (`restore_expirations` / `restore_cidr_blocks`), plus the
/// snapshot LSN for the WAL tail replay. Expired snapshot entries are
/// dropped here (they must not resurrect as live blocks).
pub fn restore_from_snapshot(store: &Store, snap: &CheckpointSnapshot) -> SnapshotRestore {
    let now_ns = now_unix_ns();
    let ram_lim = store.get_stats().ram_limit_mb.max(1) * 1024 * 1024;
    let mut ip_expirations = Vec::new();
    let mut cidr_expirations = Vec::new();
    for snap_ip in snap.blocked_ips.iter() {
        match classify(snap_ip.expires_at_ns, now_ns) {
            ExpiryVerdict::Expired => continue, // dead before recovery — drop
            verdict => {
                let rec = StorageIpRecord {
                    ip: snap_ip.ip,
                    request_count: 0,
                    ewma_rps: 0.0,
                    cusum_s: 0.0,
                    baseline_rps: 0.0,
                    prev_sample_hot: false,
                    sample_count: 0,
                    pulse_samples_in_window: 0,
                    pulse_window_start_ns: 0,
                    first_seen_ns: snap_ip.since_ns,
                    last_seen_ns: snap_ip.since_ns,
                    bytes_in: 0,
                    status_dist: [0; 5],
                    proto_fingerprint: 0,
                    threat_score: 0.0,
                    block_state: StorageBlockState::Blocked {
                        reason: BlockReason::from_reason_str(&snap_ip.reason)
                            .unwrap_or(BlockReason::ManualBlock),
                        since_ns: snap_ip.since_ns,
                    },
                };
                // Store-level TTL (secs) for its passive expiry wheel; the
                // enforcement ring re-arms below from the same deadline.
                let ttl_secs = match verdict {
                    ExpiryVerdict::Live { remaining_ns } => {
                        let secs = remaining_ns / 1_000_000_000;
                        ip_expirations.push((snap_ip.ip, remaining_ns));
                        Some(secs.max(1)) // never restore an already-dead wheel card
                    }
                    _ => None,
                };
                let _ = store.insert(snap_ip.ip, Value::IpRecord(rec), ttl_secs, ram_lim);
            }
        }
    }
    for c in snap.active_cidrs.iter() {
        match classify(c.expires_at_ns, now_ns) {
            ExpiryVerdict::Expired => continue,
            verdict => {
                store.active_cidrs.insert(c.network, ());
                if let ExpiryVerdict::Live { remaining_ns } = verdict {
                    cidr_expirations.push((c.network, remaining_ns));
                }
            }
        }
    }
    SnapshotRestore {
        ip_expirations,
        cidr_expirations,
        lsn: snap.lsn,
    }
}

/// Expiration schedules + LSN handed back by `restore_from_snapshot`.
pub struct SnapshotRestore {
    /// (ip, remaining_ns) — enforcement re-arms via restore_expirations_ns.
    pub ip_expirations: Vec<(IpAddr, u64)>,
    /// (network, remaining_ns) — enforcement re-arms via restore_cidr_blocks_ns.
    pub cidr_expirations: Vec<(IpNetwork, u64)>,
    /// Snapshot LSN — WAL tail replay starts after it.
    pub lsn: u64,
}

/// Convert an absolute Unix-ns deadline back into a monotonic Instant for
/// enforcement's TTL ring. Checked: a deadline before "now" clamps to now.
pub fn instant_from_unix_ns(deadline_ns: u64, now_unix_ns: u64) -> std::time::Instant {
    let ahead_ns = deadline_ns.saturating_sub(now_unix_ns);
    std::time::Instant::now() + std::time::Duration::from_nanos(ahead_ns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net(oct: [u8; 4]) -> IpNetwork {
        IpNetwork::new(IpAddr::from(oct), 24).unwrap()
    }

    #[test]
    fn classify_verdicts() {
        let now = 1_000_000_000u64;
        assert_eq!(classify(None, now), ExpiryVerdict::Permanent);
        assert_eq!(
            classify(Some(now + 5), now),
            ExpiryVerdict::Live { remaining_ns: 5 }
        );
        assert_eq!(classify(Some(now), now), ExpiryVerdict::Expired);
        assert_eq!(classify(Some(now - 5), now), ExpiryVerdict::Expired);
    }

    #[test]
    fn snapshot_roundtrip_preserves_deadlines() {
        let store = Store::new(16);
        let ip = IpAddr::from([10, 4, 4, 4]);
        let deadline = now_unix_ns() + 30_000_000_000;
        let mut ip_exp = std::collections::HashMap::new();
        ip_exp.insert(ip, deadline);
        let state = CheckpointState {
            ip_expirations: ip_exp,
            cidrs: vec![CidrSnapshot {
                network: net([172, 16, 9, 0]),
                expires_at_ns: None,
            }],
        };
        // Seed a blocked record.
        let rec = StorageIpRecord {
            ip,
            request_count: 0,
            ewma_rps: 0.0,
            cusum_s: 0.0,
            baseline_rps: 0.0,
            prev_sample_hot: false,
            sample_count: 0,
            pulse_samples_in_window: 0,
            pulse_window_start_ns: 0,
            first_seen_ns: now_unix_ns(),
            last_seen_ns: now_unix_ns(),
            bytes_in: 0,
            status_dist: [0; 5],
            proto_fingerprint: 0,
            threat_score: 0.0,
            block_state: StorageBlockState::Blocked {
                reason: BlockReason::ManualBlock,
                since_ns: now_unix_ns(),
            },
        };
        store
            .insert(ip, Value::IpRecord(rec), None, 256 * 1024 * 1024)
            .unwrap();
        let snap = build_snapshot(&store, &state, 42);
        assert_eq!(snap.blocked_ips[0].expires_at_ns, Some(deadline));
        assert_eq!(snap.active_cidrs.len(), 1);
        assert_eq!(snap.active_cidrs[0].expires_at_ns, None);

        // Restore into a fresh store.
        let store2 = Store::new(16);
        let restored = restore_from_snapshot(&store2, &snap);
        assert!(store2.get(&ip).unwrap().is_blocked());
        assert!(store2.active_cidrs.get(&net([172, 16, 9, 0])).is_some());
        assert_eq!(restored.lsn, 42);
        // Remaining ≈ 30s minus test time (bounded, generous tolerance).
        let (_, rem) = restored.ip_expirations[0];
        assert!(rem > 29_000_000_000 && rem <= 30_000_000_000, "rem={rem}");
        assert!(
            restored.cidr_expirations.is_empty(),
            "permanent CIDR has no deadline"
        );
    }

    #[test]
    fn expired_snapshot_blocks_are_dropped() {
        let store = Store::new(16);
        let ip = IpAddr::from([10, 5, 5, 5]);
        let past = now_unix_ns().saturating_sub(10_000_000_000);
        let snap = CheckpointSnapshot {
            lsn: 7,
            ts_ns: past,
            blocked_ips: vec![IpBlockSnapshot {
                ip,
                reason: "manual".into(),
                since_ns: past,
                expires_at_ns: Some(past + 2_000_000_000), // expired 8s ago
            }],
            active_cidrs: vec![],
        };
        let restored = restore_from_snapshot(&store, &snap);
        assert!(store.get(&ip).is_none(), "expired block must not resurrect");
        assert!(restored.ip_expirations.is_empty());
    }

    #[test]
    fn build_snapshot_without_deadline_is_permanent() {
        let store = Store::new(16);
        let ip = IpAddr::from([10, 6, 6, 6]);
        let rec = StorageIpRecord {
            ip,
            request_count: 0,
            ewma_rps: 0.0,
            cusum_s: 0.0,
            baseline_rps: 0.0,
            prev_sample_hot: false,
            sample_count: 0,
            pulse_samples_in_window: 0,
            pulse_window_start_ns: 0,
            first_seen_ns: now_unix_ns(),
            last_seen_ns: now_unix_ns(),
            bytes_in: 0,
            status_dist: [0; 5],
            proto_fingerprint: 0,
            threat_score: 0.0,
            block_state: StorageBlockState::Blocked {
                reason: BlockReason::ManualBlock,
                since_ns: now_unix_ns(),
            },
        };
        store
            .insert(ip, Value::IpRecord(rec), None, 256 * 1024 * 1024)
            .unwrap();
        let state = CheckpointState::default();
        let snap = build_snapshot(&store, &state, 1);
        assert_eq!(snap.blocked_ips[0].expires_at_ns, None);
        let store2 = Store::new(16);
        let restored = restore_from_snapshot(&store2, &snap);
        assert!(store2.get(&ip).unwrap().is_blocked());
        assert!(restored.ip_expirations.is_empty(), "permanent → no re-arm");
    }
}
