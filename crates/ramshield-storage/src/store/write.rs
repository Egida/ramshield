use crate::*;

impl Store {
    /// Insert with RAM limit enforcement. Only enforces limit on net-new growth,
    /// allowing replacement of existing entries without triggering capacity errors.
    /// Capacity semantics: byte-accounted via
    /// heap_bytes delta, errors as Result (never panic/log-and-continue).
    pub fn insert(
        &self,
        key: IpAddr,
        value: Value,
        ttl_secs: Option<u64>,
        ram_limit_bytes: usize,
    ) -> Result<()> {
        let expires_at = ttl_secs.map(|s| Instant::now() + Duration::from_secs(s));
        let new_has_ttl = expires_at.is_some();
        let new_entry = Entry { value, expires_at };
        let new_blocked = new_entry.value.is_blocked();
        let entry_size = std::mem::size_of::<IpAddr>()
            + std::mem::size_of::<Entry>()
            + new_entry.value.heap_bytes();

        // Shard-locked commit (entry API): the shard write lock is held
        // across the capacity check AND the publication, so
        //  - a rejected fresh insert is NEVER observable — the old flow
        //    published via inner.insert() and rolled back with remove(),
        //    leaving a window where a concurrent get() saw a phantom entry;
        //  - RAM accounting changes in the same critical section as the
        //    value it accounts for;
        //  - a replacement swaps the LIVE value under the lock (there is no
        //    stale get()-snapshot to race), so a late commit cannot roll
        //    back a newer value: block state transitions can never be
        //    clobbered by an older copy.
        match self.inner.entry(key) {
            dashmap::Entry::Occupied(mut o) => {
                // Replacement: swap in place. net_growth may be negative
                // (smaller value) — the limit only gates net-new growth, so
                // adjust the counters directly.
                let old = std::mem::replace(
                    o.get_mut(),
                    Entry {
                        value: new_entry.value,
                        expires_at,
                    },
                );
                let old_size = std::mem::size_of::<Entry>()
                    + old.value.heap_bytes()
                    + std::mem::size_of::<IpAddr>();
                let was_blocked = old.value.is_blocked();
                let old_had_ttl = old.expires_at.is_some();
                let net_growth = entry_size.saturating_sub(old_size);
                if entry_size >= old_size {
                    self.ram_bytes
                        .fetch_add(entry_size - old_size, Ordering::Relaxed);
                } else {
                    self.ram_bytes
                        .fetch_sub(old_size - entry_size, Ordering::Relaxed);
                }
                // Blocked index updates while still holding the shard lock.
                // Must be before `drop(o)` to prevent races with concurrent
                // unblock/evict that would observe inconsistent state.
                if !was_blocked && new_blocked {
                    self.blocked_count.fetch_add(1, Ordering::Relaxed);
                    self.blocked_set.insert(key, ());
                } else if was_blocked && !new_blocked {
                    self.blocked_count.fetch_sub(1, Ordering::Relaxed);
                    self.blocked_set.remove(&key);
                }
                match (old_had_ttl, new_has_ttl) {
                    (false, true) => {
                        self.ttl_entries.fetch_add(1, Ordering::Relaxed);
                    }
                    (true, false) => {
                        self.ttl_entries.fetch_sub(1, Ordering::Relaxed);
                    }
                    _ => {}
                }
                if tracing::enabled!(tracing::Level::TRACE) {
                    let current = self.ram_bytes.load(Ordering::Relaxed);
                    tracing::trace!(
                        ram_bytes = current,
                        net_growth,
                        key = %key,
                        "store insert accounted"
                    );
                }
                if entry_size >= old_size {
                    self.traffic
                        .used_bytes
                        .fetch_add((entry_size - old_size) as u64, Ordering::Relaxed);
                } else {
                    self.traffic
                        .used_bytes
                        .fetch_sub((old_size - entry_size) as u64, Ordering::Relaxed);
                }
                self.total_inserts.fetch_add(1, Ordering::Relaxed);
                if tracing::enabled!(tracing::Level::TRACE) {
                    tracing::trace!(key = %key, total_inserts = self.total_inserts.load(Ordering::Relaxed), "store insert committed");
                }
                drop(o); // lock released after all bookkeeping
                Ok(())
            }
            dashmap::Entry::Vacant(v) => {
                // Fresh insert: reserve capacity atomically BEFORE the entry
                // exists. Capacity check must be atomic: two threads both
                // reading the pre-insert `ram_bytes` and both deciding "fits"
                // will both exceed the budget — the CAS serializes them.
                let mut current = self.ram_bytes.load(Ordering::Relaxed);
                loop {
                    if current + entry_size > ram_limit_bytes {
                        tracing::warn!(
                            key = %key,
                            limit_mb = ram_limit_bytes / (1024 * 1024),
                            "store insert rejected: capacity exceeded"
                        );
                        return Err(RsError::CapacityExceeded {
                            limit_mb: ram_limit_bytes / (1024 * 1024),
                        });
                    }
                    match self.ram_bytes.compare_exchange_weak(
                        current,
                        current + entry_size,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => break,
                        Err(observed) => current = observed,
                    }
                }
                v.insert(new_entry);
                if new_blocked {
                    self.blocked_count.fetch_add(1, Ordering::Relaxed);
                    self.blocked_set.insert(key, ());
                }
                if new_has_ttl {
                    self.ttl_entries.fetch_add(1, Ordering::Relaxed);
                }
                self.traffic
                    .used_bytes
                    .fetch_add(entry_size as u64, Ordering::Relaxed);
                self.total_inserts.fetch_add(1, Ordering::Relaxed);
                if tracing::enabled!(tracing::Level::TRACE) {
                    let current = self.ram_bytes.load(Ordering::Relaxed);
                    tracing::trace!(
                        ram_bytes = current,
                        net_growth = entry_size,
                        key = %key,
                        "store insert accounted"
                    );
                }
                Ok(())
            }
        }
    }

    /// Atomic read-modify-write of an `IpRecord` under ONE shard lock.
    ///
    /// P0 fix (round-4 Q3): `merge_record` used `get()` -> clone -> mutate ->
    /// `insert()` — two lock acquisitions. If the enforcement actor blocked an
    /// IP inside that window, the flusher's stale snapshot (captured while
    /// still `Clean`) re-inserted over the fresh `Blocked` state: active
    /// attacker resurrected, exactly under flood conditions. The reverse
    /// order also lost stat updates to last-writer-wins.
    ///
    /// Contract: `f` mutates only stat fields — it MUST NOT touch
    /// `block_state` (that authority lives with the enforcement actor, which
    /// keeps using `insert`). Because the record is mutated in place, other
    /// writers' block transitions can never be clobbered by a stale copy.
    ///
    /// Returns `(f's result, stored)` — `stored=false` when a genuinely new
    /// key was refused by the RAM budget (the mutated record is dropped;
    /// matches `insert`'s CapacityExceeded behavior, which also only ever
    /// rejects net-new keys).
    pub fn update_ip<R>(
        &self,
        key: IpAddr,
        default: IpRecord,
        ram_limit_bytes: usize,
        f: impl FnOnce(&mut IpRecord) -> R,
    ) -> (R, bool) {
        match self.inner.entry(key) {
            dashmap::Entry::Occupied(mut o) => {
                let e = o.get_mut();
                // Live IpRecord: mutate in place, zero accounting changes
                // (IpRecord is fixed-size, heap_bytes() == 0 by design).
                if !e.is_expired()
                    && let Value::IpRecord(rec) = &mut e.value
                {
                    return (f(rec), true);
                }
                // Occupied but expired / wrong variant: replace in place with
                // the same bookkeeping insert() does for a replacement.
                let (was_blocked, old_size) = {
                    let old_value = std::mem::replace(&mut e.value, Value::Counter(0));
                    let wb = old_value.is_blocked();
                    let os = std::mem::size_of::<Entry>()
                        + old_value.heap_bytes()
                        + std::mem::size_of::<IpAddr>();
                    (wb, os)
                };
                let mut rec = default;
                let out = f(&mut rec);
                let old_had_ttl = e.expires_at.is_some();
                e.value = Value::IpRecord(rec);
                e.expires_at = None;
                if old_had_ttl {
                    // Replaced a ttl-bearing entry with a permanent one.
                    self.ttl_entries.fetch_sub(1, Ordering::Relaxed);
                }
                // Blocked index update while still holding shard lock.
                if was_blocked {
                    self.blocked_count.fetch_sub(1, Ordering::Relaxed);
                    self.blocked_set.remove(&key);
                }
                let new_size = std::mem::size_of::<Entry>() + std::mem::size_of::<IpAddr>();
                drop(o); // release shard lock after all bookkeeping
                self.apply_growth_size_only(old_size, new_size);
                (out, true)
            }
            dashmap::Entry::Vacant(v) => {
                let mut rec = default;
                let out = f(&mut rec);
                let entry_size = std::mem::size_of::<IpAddr>() + std::mem::size_of::<Entry>(); // heap 0, Clean
                // Net-new capacity gate: same CAS loop as insert()'s fresh path.
                let mut current = self.ram_bytes.load(Ordering::Relaxed);
                loop {
                    if current + entry_size > ram_limit_bytes {
                        tracing::warn!("Store::update_ip - CapacityExceeded for key: {}", key);
                        return (out, false);
                    }
                    match self.ram_bytes.compare_exchange_weak(
                        current,
                        current + entry_size,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => break,
                        Err(observed) => current = observed,
                    }
                }
                v.insert(Entry {
                    value: Value::IpRecord(rec),
                    expires_at: None,
                });
                self.traffic
                    .used_bytes
                    .fetch_add(entry_size as u64, Ordering::Relaxed);
                self.total_inserts.fetch_add(1, Ordering::Relaxed);
                (out, true)
            }
        }
    }
}
