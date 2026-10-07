use crate::*;

impl Store {
    /// Size-only variant for callers that manage blocked index updates
    /// inside the DashMap shard lock (before drop).
    pub(crate) fn apply_growth_size_only(&self, old_size: usize, new_size: usize) {
        if new_size >= old_size {
            self.ram_bytes
                .fetch_add(new_size - old_size, Ordering::Relaxed);
            self.traffic
                .used_bytes
                .fetch_add((new_size - old_size) as u64, Ordering::Relaxed);
        } else {
            self.ram_bytes
                .fetch_sub(old_size - new_size, Ordering::Relaxed);
            self.traffic
                .used_bytes
                .fetch_sub((old_size - new_size) as u64, Ordering::Relaxed);
        }
        self.total_inserts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn evict_batch(&self, keys: &[IpAddr]) {
        // ponytail: `entry()` gives exclusive shard lock once per key
        for key in keys {
            if let dashmap::Entry::Occupied(e) = self.inner.entry(*key)
                && e.get().is_expired()
            {
                let was_blocked = e.get().value.is_blocked();
                let had_ttl = e.get().expires_at.is_some();
                let (_, removed) = e.remove_entry();
                if had_ttl {
                    self.ttl_entries.fetch_sub(1, Ordering::Relaxed);
                }
                let freed = std::mem::size_of::<IpAddr>()
                    + std::mem::size_of::<Entry>()
                    + removed.value.heap_bytes();
                self.ram_bytes.fetch_sub(freed, Ordering::Relaxed);
                self.traffic
                    .used_bytes
                    .fetch_sub(freed as u64, Ordering::Relaxed);
                self.total_evictions.fetch_add(1, Ordering::Relaxed);
                if was_blocked {
                    self.blocked_count.fetch_sub(1, Ordering::Relaxed);
                    self.blocked_set.remove(key);
                }
                // P0 fix: stale subnet_index entries leaked forever.
                self.update_subnet_index(*key, subnet_key_u128(*key), true);
            }
        }
    }

    /// Sweep all entries and remove expired ones. Returns count of evicted entries.
    /// RAM-for-CPU item 14: O(1) early return when nothing carries a TTL —
    /// the production case (block TTLs live in the enforcement actor; store
    /// inserts pass None). Without the gate this was a full-store key
    /// collection + per-key rehash every 60s to evict exactly zero.
    pub fn evict_expired(&self) -> usize {
        if self.ttl_entries.load(Ordering::Relaxed) == 0 {
            return 0;
        }
        let before = self.inner.len();
        let keys: Vec<IpAddr> = self.inner.iter().map(|e| *e.key()).collect();
        self.evict_batch(&keys);
        before.saturating_sub(self.inner.len())
    }

    pub fn ram_bytes(&self) -> usize {
        self.ram_bytes.load(Ordering::Relaxed)
    }

    #[doc(hidden)]
    pub fn set_ram_limit_mb_for_testing(&self, mb: usize) {
        self.traffic.ram_limit_mb.store(mb, Ordering::Relaxed);
    }

    #[doc(hidden)]
    pub fn set_ram_bytes_for_testing(&self, bytes: usize) {
        self.ram_bytes.store(bytes, Ordering::Relaxed);
    }
}
