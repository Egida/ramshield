//! Checkpoint snapshot: durable image of enforcement state at an LSN.
//! Written by the engine on a periodic loop; loaded at boot to skip
//! replaying historical WAL entries before the checkpoint boundary.

use ramshield_storage::{Store, IpRecord as StorageIpRecord, BlockState as StorageBlockState, Value};
use ramshield_types::{BlockReason, IpNetwork, Result, RsError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    /// 0 / None → permanent block.
    pub ttl_secs: Option<u64>,
}

/// Snapshot of enforcement state at a given LSN.
/// On recovery: load snapshot, then replay WAL entries > lsn.
#[derive(Debug, Serialize, Deserialize)]
pub struct CheckpointSnapshot {
    pub lsn: u64,
    pub ts_ns: u64,
    pub blocked_ips: Vec<IpBlockSnapshot>,
    pub active_cidrs: Vec<(IpNetwork, u64)>,
}

/// Build snapshot path from WAL base dir and LSN.
pub fn snapshot_path(wal_dir: &str, lsn: u64) -> String {
    PathBuf::from(wal_dir)
        .join(format!("snapshot.{lsn:020}.ckpt"))
        .to_string_lossy()
        .to_string()
}

/// Write snapshot atomically: tmp + fsync + rename.
pub fn write_snapshot(dir: &str, snap: &CheckpointSnapshot) -> Result<String> {
    let path = snapshot_path(dir, snap.lsn);
    let tmp = format!("{path}.tmp");
    let json = serde_json::to_vec(snap)
        .map_err(|e| RsError::Serde(e.to_string()))?;
    {
        let mut f = File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    fsync_dir(dir)?;
    Ok(path)
}

/// Load snapshot from file. Returns Ok(None) when file does not exist or is
/// obviously absent (missing file). Corrupt content returns an error — the
/// caller decides whether to fall back to full WAL replay.
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

/// Build a snapshot from the live store and CIDR map.
/// Lifted after a Wal::checkpoint() so LSN is current.
pub fn build_snapshot(
    store: &Store,
    cidrs: &HashMap<IpNetwork, u64>,
    lsn: u64,
) -> CheckpointSnapshot {
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
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
            let ttl_secs: Option<u64> = None; // WAL tail replay re-arms TTL
            Some(IpBlockSnapshot {
                ip,
                reason: reason.as_str().to_string(),
                since_ns,
                ttl_secs,
            })
        })
        .collect();
    let active_cidrs: Vec<(IpNetwork, u64)> = cidrs
        .iter()
        .map(|(net, secs)| (*net, *secs))
        .collect();
    CheckpointSnapshot {
        lsn,
        ts_ns: now_ns,
        blocked_ips,
        active_cidrs,
    }
}

/// Restore store + CIDR map from a snapshot.
/// Returns the snapshot LSN for WAL tail replay.
pub fn restore_from_snapshot(
    store: &Store,
    cidrs: &mut HashMap<IpNetwork, u64>,
    snap: &CheckpointSnapshot,
) -> u64 {
    let ram_lim = store.get_stats().ram_limit_mb.max(1) * 1024 * 1024;
    for snap_ip in snap.blocked_ips.iter() {
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
                reason: BlockReason::from_reason_str(&snap_ip.reason).unwrap_or(BlockReason::ManualBlock),
                since_ns: snap_ip.since_ns,
            },
        };
        let _ = store.insert(
            snap_ip.ip,
            Value::IpRecord(rec),
            snap_ip.ttl_secs,
            ram_lim,
        );
    }
    for (cidr, ttl) in snap.active_cidrs.iter() {
        cidrs.insert(*cidr, *ttl);
    }
    snap.lsn
}