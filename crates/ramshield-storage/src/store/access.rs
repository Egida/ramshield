use crate::*;

impl Store {
    pub fn get(&self, key: &IpAddr) -> Option<Value> {
        let entry = self.inner.get(key)?;
        if entry.is_expired() {
            return None;
        }
        Some(entry.value.clone())
    }

    /// Is this IP blocked by an active CIDR rule?
    ///
    /// The CIDR decision is the block; member hosts are views of it. A member
    /// of a blocked /24 has no `IpRecord.block_state` of its own, so a query
    /// that reads only per-IP state reports `blocked:false` for an address the
    /// dataplane is dropping. `active_cidrs` is written solely by the
    /// enforcement actor, so this is the same clock that gates the kernel.
    ///
    /// ponytail: linear scan over active CIDR blocks — the set is bounded by
    /// subnet decisions (tens, not thousands). Upgrade to an LPM trie if a
    /// deployment ever holds thousands of concurrent prefix blocks.
    pub fn is_blocked_by_cidr(&self, ip: &IpAddr) -> Option<IpNetwork> {
        self.active_cidrs
            .iter()
            .map(|e| *e.key())
            .find(|net| net.contains(*ip))
    }

    pub fn remove(&self, key: &IpAddr) -> Option<Value> {
        self.inner.remove(key).map(|(_k, e)| {
            if e.expires_at.is_some() {
                self.ttl_entries.fetch_sub(1, Ordering::Relaxed);
            }
            let freed =
                std::mem::size_of::<IpAddr>() + std::mem::size_of::<Entry>() + e.value.heap_bytes();
            self.ram_bytes.fetch_sub(freed, Ordering::Relaxed);
            self.traffic
                .used_bytes
                .fetch_sub(freed as u64, Ordering::Relaxed);
            self.total_evictions.fetch_add(1, Ordering::Relaxed);
            if e.value.is_blocked() {
                self.blocked_count.fetch_sub(1, Ordering::Relaxed);
                self.blocked_set.remove(key);
            }
            // P0 fix: subnet_index must be cleaned here too, same reason
            // as evict_batch — stale entries leak without this.
            self.update_subnet_index(*key, subnet_key_u128(*key), true);
            e.value
        })
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn inner(&self) -> &IpEntryMap {
        &self.inner
    }

    pub fn subnet_table(&self) -> &SubnetTable {
        &self.subnet_table
    }

    /// Returns aggregate store statistics for dashboard / CLI.
    pub fn get_stats(&self) -> StoreStats {
        let ips_tracked = self.inner.len();
        // ponytail: O(1) via atomic counter — no O(store) scan.
        let blocked = self.blocked_count.load(Ordering::Relaxed);
        let ram_bytes = self.ram_bytes.load(Ordering::Relaxed);
        let ram_limit_mb = self.traffic.ram_limit_mb.load(Ordering::Relaxed);
        let uptime_secs = self.traffic.uptime_secs.load(Ordering::Relaxed);
        let evictions = self.total_evictions.load(Ordering::Relaxed);
        StoreStats {
            ips_tracked,
            blocked,
            ram_bytes,
            ram_limit_mb,
            uptime_secs,
            evictions,
        }
    }

    /// Get all currently blocked IPs for XDP reconciliation.
    /// O(B) via the `blocked_set` index — maintained on every block/unblock
    /// transition in `insert`, `remove`, and `evict_batch`.
    /// B = number of blocked IPs, NOT total store size.
    pub fn get_all_blocked_ips(&self) -> Vec<IpAddr> {
        self.blocked_set.iter().map(|e| *e.key()).collect()
    }
}
