use crate::*;

impl EnforcementService {
    /// Snapshot CIDR state for checkpoint: extracts absolute deadline from
    /// the active expiration schedule. Permanent CIDRs (no expiration) are
    /// emitted as `expires_at_ns: None`.
    pub fn checkpoint_cidr_state(&self) -> Vec<CidrSnapshot> {
        let now_unix_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        self.store
            .active_cidrs
            .iter()
            .map(|e| *e.key())
            .map(|network| CidrSnapshot {
                network,
                expires_at_ns: self
                    .cidr_expirations
                    .get(&network)
                    .map(|&deadline| unix_ns_from_instant(deadline, now_unix_ns)),
            })
            .collect()
    }

    pub fn remove_restored_cidr(&mut self, network: IpNetwork) {
        self.store.active_cidrs.remove(&network);
        self.cidr_expirations.remove(&network);
        if let Some(shared) = &self.checkpoint_shared {
            shared.remove_cidr_expiration(&network);
        }
        if let Err(e) = self.xdp.apply_cidr_unblock(network, Uuid::new_v4()) {
            warn!(cidr=?network, "WAL CIDR restore: userspace removed, XDP unblock failed: {}", e);
        }
    }

    pub fn restore_cidr_blocks(&mut self, pairs: impl IntoIterator<Item = (IpNetwork, u64)>) {
        for (network, remaining_secs) in pairs {
            // Authoritative state lives in userspace (active_cidrs) regardless
            // of the XDP projection result. XDP failure must not destroy the
            // durable security decision — reconciliation retries later.
            self.store.active_cidrs.insert(network, ());
            let at = Instant::now() + Duration::from_secs(remaining_secs);
            if remaining_secs > 0 {
                self.cidr_expirations.insert(network, at);
                if let Some(shared) = &self.checkpoint_shared {
                    let now_unix_ns = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0);
                    shared.set_cidr_expiration(network, unix_ns_from_instant(at, now_unix_ns));
                }
            } else if let Some(shared) = &self.checkpoint_shared {
                shared.remove_cidr_expiration(&network);
            }
            if let Err(e) = self
                .xdp
                .apply_cidr_block(network, Uuid::new_v4(), remaining_secs)
            {
                warn!(cidr=?network, "WAL CIDR restore: userspace active, XDP apply failed: {}", e);
            }
        }
    }
}
