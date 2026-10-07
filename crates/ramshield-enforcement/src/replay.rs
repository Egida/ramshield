use super::*;

/// Replay WAL entries into the store: fold BlockIp/UnblockIp in LSN order to
/// the final block set, skipping blocks whose TTL already elapsed. Returns
/// the still-live blocks as `(ip, ttl_secs)` pairs — the caller re-arms the
/// enforcement TTL schedule with them (P1-4: blocks restored WITHOUT an
/// expiry card never expire; expirations/buckets are empty at boot).
/// Call before `run()` so the XDP reconciliation inside it picks the
/// recovered state up.
pub fn replay_wal_into_store(
    store: &Arc<Store>,
    wal: &Wal,
    min_lsn: u64,
) -> anyhow::Result<Vec<(IpAddr, u64)>> {
    replay_wal_into_store_seeded(store, wal, min_lsn, std::collections::HashMap::new())
}

/// Checkpoint-accelerated variant: the fold STARTS from `seed` (snapshot
/// block state: ip → (reason, since_ns, deadline_ns)) so an IP blocked in
/// the snapshot and untouched by the tail remains blocked, while a tail
/// UnblockIp correctly removes it. Tail blocks/Unblocks then fold on top.
pub fn replay_wal_into_store_seeded(
    store: &Arc<Store>,
    wal: &Wal,
    min_lsn: u64,
    seed: std::collections::HashMap<IpAddr, (String, u64, u64)>,
) -> anyhow::Result<Vec<(IpAddr, u64)>> {
    let entries = if min_lsn > 0 {
        wal.replay_from(min_lsn)?
    } else {
        wal.replay()?
    };
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    let seed_ips: Vec<IpAddr> = seed.keys().copied().collect();
    // Keep absolute deadlines throughout the fold. Converting a snapshot
    // deadline to whole seconds here loses precision and can resurrect a
    // nearly-expired block. Only convert to remaining seconds at the final
    // scheduler hand-off.
    let mut blocked: std::collections::HashMap<IpAddr, (BlockReason, u64, Option<u64>)> = seed
        .into_iter()
        .map(|(ip, (reason, since, deadline_ns))| {
            let deadline = (deadline_ns > 0).then_some(deadline_ns);
            (ip, (reason_to_block_reason(&reason), since, deadline))
        })
        .collect();
    for entry in entries {
        match entry {
            WalEntry::BlockIp {
                ip,
                reason,
                ttl_secs,
                ts_ns,
            } => {
                if let Ok(ip) = ip.parse() {
                    let deadline = ttl_secs
                        .map(|secs| ts_ns.saturating_add(secs.saturating_mul(1_000_000_000)));
                    blocked.insert(ip, (reason_to_block_reason(&reason), ts_ns, deadline));
                }
            }
            WalEntry::UnblockIp { ip, .. } => {
                if let Ok(ip) = ip.parse() {
                    blocked.remove(&ip);
                }
            }
            _ => {} // Insert/Delete/Checkpoint: traffic data, not block state
        }
    }

    let ram_lim = store.traffic.ram_limit_mb.load(Ordering::Relaxed).max(1) * 1024 * 1024;
    // Seed reconciliation: snapshot IPs that the tail UNBLOCKED must have their
    // store records (inserted by restore_from_snapshot) removed — the fold's
    // final loop only touches surviving entries, so without this a tail unblock
    // of a snapshot IP would leave a phantom block.
    for seed_ip in seed_ips {
        if !blocked.contains_key(&seed_ip) {
            store.remove(&seed_ip);
        }
    }
    let mut restored: Vec<(IpAddr, u64)> = Vec::new();
    for (ip, (reason, ts_ns, deadline_ns)) in blocked {
        // Expired absolute deadline → don't resurrect.
        if let Some(deadline) = deadline_ns
            && deadline <= now_ns
        {
            continue;
        }
        let rec = match store.get(&ip) {
            Some(Value::IpRecord(mut r)) => {
                r.block_state = BlockState::Blocked {
                    reason,
                    since_ns: ts_ns,
                };
                r
            }
            _ => IpRecord {
                ip,
                request_count: 0,
                ewma_rps: 0.0,
                cusum_s: 0.0,
                baseline_rps: 0.0,
                prev_sample_hot: false,
                sample_count: 0,
                relative_breach_streak: 0,
                pulse_samples_in_window: 0,
                pulse_window_start_ns: 0,
                first_seen_ns: ts_ns,
                last_seen_ns: ts_ns,
                bytes_in: 0,
                status_dist: [0; 5],
                proto_fingerprint: 0,
                threat_score: 0.0,
                block_state: BlockState::Blocked {
                    reason,
                    since_ns: ts_ns,
                },
            },
        };
        store
            .insert(ip, Value::IpRecord(rec), None, ram_lim)
            .map_err(|e| anyhow::anyhow!("WAL replay insert {ip}: {e}"))?;
        // Scheduler APIs use whole seconds, so round UP only at this final
        // boundary. The authoritative expiration check above used the exact
        // absolute deadline. This avoids extending a block during state fold.
        let remaining = deadline_ns
            .map(|deadline| deadline.saturating_sub(now_ns).div_ceil(1_000_000_000))
            .unwrap_or(0);
        restored.push((ip, remaining));
    }
    info!(
        "WAL replay: restored {} live blocks ({} with TTL)",
        restored.len(),
        restored.iter().filter(|(_, t)| *t > 0).count()
    );
    Ok(restored)
}

pub fn replay_wal_cidrs(wal: &Wal) -> anyhow::Result<Vec<(IpNetwork, u64)>> {
    replay_wal_cidrs_seeded(wal, 0, std::collections::HashMap::new()).map(|r| r.0)
}

pub fn replay_wal_cidrs_from(wal: &Wal, min_lsn: u64) -> anyhow::Result<Vec<(IpNetwork, u64)>> {
    replay_wal_cidrs_seeded(wal, min_lsn, std::collections::HashMap::new()).map(|r| r.0)
}

/// Replay CIDR state from a checkpoint seed plus the WAL tail. The returned
/// second value is the final authoritative CIDR set; callers restoring a
/// snapshot must remove seeded CIDRs absent from this set before re-arming
/// the XDP projection. Deadlines remain absolute until the final scheduler
/// hand-off, matching the IP recovery path.
#[allow(clippy::type_complexity)] // ponytail: two parallel collections stay local; alias if callers grow.
pub fn replay_wal_cidrs_seeded(
    wal: &Wal,
    min_lsn: u64,
    seed: std::collections::HashMap<IpNetwork, Option<u64>>,
) -> anyhow::Result<(Vec<(IpNetwork, u64)>, std::collections::HashSet<IpNetwork>)> {
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut blocked: std::collections::HashMap<IpNetwork, (u64, Option<u64>)> = seed
        .into_iter()
        .filter(|(_, deadline)| deadline.map(|d| d > now_ns).unwrap_or(true))
        .map(|(network, deadline)| (network, (0, deadline)))
        .collect();
    let entries = if min_lsn > 0 {
        wal.replay_from(min_lsn)?
    } else {
        wal.replay()?
    };
    for entry in entries {
        match entry {
            WalEntry::BlockCidr {
                cidr,
                ttl_secs,
                ts_ns,
                ..
            } => {
                let deadline =
                    ttl_secs.map(|secs| ts_ns.saturating_add(secs.saturating_mul(1_000_000_000)));
                blocked.insert(cidr, (ts_ns, deadline));
            }
            WalEntry::UnblockCidr { cidr, .. } => {
                blocked.remove(&cidr);
            }
            _ => {}
        }
    }

    let mut final_cidrs = std::collections::HashSet::new();
    let mut restored = Vec::new();
    for (cidr, (_ts_ns, deadline)) in blocked {
        if let Some(deadline) = deadline {
            if deadline <= now_ns {
                continue;
            }
            final_cidrs.insert(cidr);
            let remaining = deadline.saturating_sub(now_ns).div_ceil(1_000_000_000);
            restored.push((cidr, remaining));
        } else {
            final_cidrs.insert(cidr);
            restored.push((cidr, 0));
        }
    }
    Ok((restored, final_cidrs))
}
