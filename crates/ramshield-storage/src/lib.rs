//! Unified RamShield storage: sharded in-memory Store (from src, perf-tuned)
//! + WAL / BlobStore durability modules (from crate).
//!
//! Types: `Value::IpRecord` is the canonical per-IP entry; subnet keys are
//! `u128` (IPv4 packed low-32, IPv6 full address) with `IpNetwork` metadata.

pub mod subnet;
pub mod wal;

pub use subnet::{subnet_key_u128, subnet_key_v4, subnet_key_v6};

use crossbeam_queue::SegQueue;
use dashmap::DashMap;
use ramshield_types::{BlockReason, IpNetwork, Result, RsError};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

pub const INLINE_MAX: usize = 64;

/// Patch B: subnet dual-gate accumulation window (ns).
///
/// One owner for a value three modules must agree on: the merge reset in
/// `merge_subnet_window` and both windowed-cardinality legs of the dual gate
/// in `ramshield-detection`. The defect this closes: at 2s, a swarm pacing
/// its pulses just past the flush cadence never held the dual gate (at least
/// `subnet_batch_threshold` hosts AND `subnet_batch_min_events` events)
/// across a boundary, because each merge landed after the previous window
/// had already been zeroed — so the detector saw only the current pulse,
/// never the aggregate.
///
/// 4s = 2x the 2s flush cadence, giving every pulse at least one full
/// window to accumulate against. Watch the upper bound when raising this:
/// the prune policy evicts on staleness (see `subnet_batch_scan`), and the
/// gate must stay comfortably shorter than that or live swarms get pruned
/// mid-attack. ponytail: fixed const, not per-subnet config — plumb to
/// Config.detection only if traffic profiles actually diverge.
pub const SUBNET_WINDOW_NS: u64 = 4 * 1_000_000_000;

/// Incremental traffic counters — updated on batch flush, read by forecasting
/// without scanning the full store (Kafka-style consumer lag / Prometheus counters).
#[derive(Debug)]
pub struct TrafficCounters {
    pub events_last_second: AtomicU64,
    pub unique_ips_window: AtomicU64,
    pub promoted_ips: AtomicU64,
    /// Subnet event counts from the latest flush window (for entropy at scale).
    /// Lock-free atomic array for concurrent reads/writes from detection and forecasting.
    pub subnet_window: [AtomicU64; 256],
    /// High-threat IPs from latest flush (bounded sample for preemptive block).
    /// Lock-free unbounded MPMC queue.
    pub threat_sample: SegQueue<(IpAddr, f32)>,
    /// RAM limit in MB from config.
    pub ram_limit_mb: AtomicUsize,
    /// Byte-precise usage tracking (crate port — complements ram_bytes estimate).
    pub used_bytes: AtomicU64,
    /// Process uptime in seconds.
    pub uptime_secs: AtomicU64,
}

impl TrafficCounters {
    pub fn new() -> Self {
        Self {
            events_last_second: AtomicU64::new(0),
            unique_ips_window: AtomicU64::new(0),
            promoted_ips: AtomicU64::new(0),
            subnet_window: std::array::from_fn(|_| AtomicU64::new(0)),
            threat_sample: SegQueue::new(),
            ram_limit_mb: AtomicUsize::new(0),
            used_bytes: AtomicU64::new(0),
            uptime_secs: AtomicU64::new(0),
        }
    }

    pub fn record_flush(&self, total_events: u64, unique_ips: u64, subnet_counts: &[u64]) {
        self.events_last_second
            .store(total_events, Ordering::Relaxed);
        self.unique_ips_window.store(unique_ips, Ordering::Relaxed);
        // Snapshot semantics: this flush's counts fully replace the previous
        // window. Only zero the slots BEYOND the incoming range — slots
        // inside the range are about to be overwritten with the new value
        // anyway. Saves ~256 atomic stores on every flush when the
        // subnet count is well under 256.
        let n = subnet_counts.len().min(256);
        for slot in &self.subnet_window[n..] {
            slot.store(0, Ordering::Relaxed);
        }
        for (i, count) in subnet_counts.iter().take(256).enumerate() {
            self.subnet_window[i].store(*count, Ordering::Relaxed);
        }
    }

    /// Push multiple threat samples into the queue.
    pub fn push_threat_samples(&self, samples: Vec<(IpAddr, f32)>) {
        for item in samples {
            // P1-9: bound the queue. The only drainer is the forecaster (may
            // be disabled/stalled); without a cap a sustained feed grows
            // ~180MB/hr. 1024 samples ≈ 8 flushes of slack — the newest
            // threat signals win, stale ones are dropped.
            if self.threat_sample.len() >= 1024 {
                break;
            }
            self.threat_sample.push(item);
        }
    }

    /// Atomically drain the threat sample queue.
    pub fn drain_threat_sample(&self) -> Vec<(IpAddr, f32)> {
        let mut sample = Vec::with_capacity(self.threat_sample.len());
        while let Some(item) = self.threat_sample.pop() {
            sample.push(item);
        }
        sample
    }
}

impl Default for TrafficCounters {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Value {
    Counter(u64),
    /// Small payloads stored as Vec<u8>.
    /// Note: we intentionally avoid [u8; 64] because serde only auto-derives
    /// fixed arrays up to [T; 32]. Vec<u8> is serde-compatible at any size.
    Inline(Vec<u8>),
    Blob(Vec<u8>),
    IpRecord(IpRecord),
    SubnetRecord(SubnetRecord),
}

impl Value {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        if bytes.len() <= INLINE_MAX {
            Value::Inline(bytes.to_vec())
        } else {
            Value::Blob(bytes.to_vec())
        }
    }

    pub fn heap_bytes(&self) -> usize {
        match self {
            Value::Inline(v) => v.len(),
            Value::Blob(v) => v.len(),
            // P1 fix: IpRecord/SubnetRecord live INSIDE the enum variant —
            // size_of::<Entry>() already accounts for them. Returning
            // size_of::<IpRecord>() here double-counted ~136 B per blocked
            // IP, starving the RAM budget to roughly half its real size.
            // Both structs are pure-value ([u32; 5], [u64; 4], fieldless
            // enums) — zero heap side-allocation, so 0 is exact.
            Value::IpRecord(_) => 0,
            Value::SubnetRecord(_) => 0,
            _ => 0,
        }
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Value::IpRecord(rec) if rec.block_state != BlockState::Clean)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpRecord {
    pub ip: IpAddr,
    pub request_count: u64,
    pub ewma_rps: f64,
    /// P1 CUSUM accumulator (Page 1954) — sustained sub-threshold drift.
    #[serde(default)]
    pub cusum_s: f64,
    /// Slow-EWMA baseline the CUSUM measures deviation from.
    #[serde(default)]
    pub baseline_rps: f64,
    /// Debounce latch: previous sample was over threshold.
    #[serde(default)]
    pub prev_sample_hot: bool,
    /// Flush samples observed (saturating) — gates CUSUM warm-up. u8 is plenty:
    /// 6 samples to arm, saturates long before overflow matters.
    #[serde(default)]
    pub sample_count: u8,
    /// Sliding-window count of distinct over-threshold batch samples.
    /// Resets on window expiry or block. ponytail: u8 caps at 255.
    #[serde(default)]
    pub pulse_samples_in_window: u8,
    /// Earliest pulse-sample timestamp in the current sliding window (ns).
    /// 0 = window not yet opened. Resets on expiry or block.
    #[serde(default)]
    pub pulse_window_start_ns: u64,
    pub first_seen_ns: u64,
    pub last_seen_ns: u64,
    pub bytes_in: u64,
    pub status_dist: [u32; 5],
    pub proto_fingerprint: u32,
    pub threat_score: f32,
    pub block_state: BlockState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BlockState {
    Clean,
    Suspicious,
    Blocked { reason: BlockReason, since_ns: u64 },
}

/// Subnet aggregate. `network` carries family-complete CIDR metadata
/// (v4 /24, v6 /64) — the single source for display (Task 1: replaces the
/// old v4-shaped `prefix: [u8;3]` that rendered v6 as garbage).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubnetRecord {
    pub network: IpNetwork,
    pub total_rps: u64,
    /// Distinct-source signal for the current window (v4 only): 256-bit map of
    /// seen host octets, 32 B flat. The real swarm signal — one abuser at 500
    /// events is a single offender; 40 distinct IPs × 12 events is an attack.
    /// v6 /64s are too large to bitmap; the batch gate reads exact
    /// `subnet_index` cardinality for them instead (IPv6 plan D1), so
    /// `unique_ips()` returning 0 only means "not the v4 fast path".
    pub host_bitmap: [u64; 4],
    pub last_updated_ns: u64,
}

impl SubnetRecord {
    #[inline]
    pub fn unique_ips(&self) -> u64 {
        self.host_bitmap.iter().map(|w| w.count_ones() as u64).sum()
    }

    #[inline]
    fn mark_host_v4(&mut self, ip: std::net::IpAddr) {
        if let std::net::IpAddr::V4(v4) = ip {
            let o = v4.octets()[3] as usize;
            self.host_bitmap[o / 64] |= 1 << (o % 64);
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub value: Value,
    pub expires_at: Option<Instant>,
}

impl Entry {
    pub fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|e| Instant::now() > e)
    }
}

/// Subnet key: IPv4 packed into low 32 bits, IPv6 full 128 bits.
pub type SubnetKey = u128;
/// ahash everywhere IPs/subnets are keys (RAM-for-CPU item 1): attacker
/// volume multiplies every SipHash. RandomState seeds per-process from the
/// OS — collision-DoS resistant, unlike fixed seeds.
type SubnetTable = DashMap<SubnetKey, SubnetRecord, ahash::RandomState>;
type IpEntryMap = DashMap<IpAddr, Entry, ahash::RandomState>;
// ponytail: plain HashSet, not the local DashSet alias — the inner set is
// only ever touched under the outer entry's shard lock, so DashMap's 32-64
// shards bought zero concurrency and cost ~3 KB per discovered subnet (the
// 2.2 state-DoS arithmetic: 100k rotated subnets ≈ 300 MB of locks for
// nothing). Same publication protocol, ~30x less memory per subnet.
type SubnetIndex =
    DashMap<SubnetKey, std::collections::HashSet<IpAddr, ahash::RandomState>, ahash::RandomState>;

pub struct Store {
    inner: Arc<IpEntryMap>,
    subnet_table: Arc<SubnetTable>,
    /// Reverse index: subnet key -> list of IPs for efficient subnet-based lookups.
    /// Maintained during batch flush to avoid O(store_size) scans.
    subnet_index: Arc<SubnetIndex>,
    /// P0 index: currently-blocked IPs. Maintained during insert/remove/evict
    /// to make `get_all_blocked_ips` O(B) instead of O(N) on XDP reconcile.
    /// B = number of blocked IPs (typically 50-200), N = total store size (100k+).
    blocked_set: Arc<DashSet<IpAddr>>,
    /// Active CIDR blocks (network prefixes enforced at the prefix level).
    /// Written ONLY by the enforcement actor — the single owner of block
    /// state. Read by `check_ip` and the dashboard so a query never reports
    /// `blocked:false` for an IP the dataplane is dropping: a member of a
    /// blocked /24 has no `IpRecord.block_state` of its own (the CIDR is the
    /// decision; members are views of it), so per-IP state alone lies.
    pub active_cidrs: Arc<DashSet<IpNetwork>>,
    ram_bytes: Arc<AtomicUsize>,
    /// O(1) blocked count — updated on BlockState transitions in insert().
    /// ponytail: does not track pre-existing blocked IPs from WAL replay unless
    /// replay calls insert() with a Blocked state (it does). add scan at boot
    /// if needed.
    blocked_count: Arc<AtomicU64>,
    pub traffic: Arc<TrafficCounters>,
    pub total_inserts: Arc<AtomicU64>,
    pub total_evictions: Arc<AtomicU64>,
    /// Count of live entries with `expires_at: Some(_)`. RAM-for-CPU item 14:
    /// production inserts pass ttl=None (block TTL lives in the enforcement
    /// actor), so the 60s `evict_expired` janitor walked the whole store to
    /// find zero. This gate makes the no-TTL case O(1). Bookkeeping is
    /// conservative by construction: increments only after a ttl-bearing
    /// insert sticks; every destroy path decrements on an observed Some.
    /// A missed decrement over-counts (scan still runs = old behavior);
    /// an over-decrement is impossible while the invariant holds.
    ttl_entries: Arc<AtomicU64>,
}

/// Minimal single-value set over DashMap (avoids pulling dashmap-set feature).
/// ahash: keys are attacker-controlled IPs/subnets — SipHash (DashMap's
/// RandomState default) burns ~20-30 cycles/hash on the per-event path and
/// invites collision probing. ahash's RandomState seeds from the OS at
/// startup (not fixed), so no key-recovery attack on the seed.
type DashSet<T> = DashMap<T, (), ahash::RandomState>;

impl Store {
    pub fn new(shard_count: usize) -> Self {
        let shards = shard_count.next_power_of_two();
        tracing::debug!("Store::new - Initializing store with {} shards", shards);
        Self {
            inner: Arc::new(IpEntryMap::with_hasher_and_shard_amount(
                ahash::RandomState::new(),
                shards,
            )),
            subnet_table: Arc::new(SubnetTable::with_hasher_and_shard_amount(
                ahash::RandomState::new(),
                32,
            )),
            subnet_index: Arc::new(SubnetIndex::with_hasher_and_shard_amount(
                ahash::RandomState::new(),
                32,
            )),
            blocked_set: Arc::new(DashSet::with_hasher_and_shard_amount(
                ahash::RandomState::new(),
                32,
            )),
            active_cidrs: Arc::new(DashSet::with_hasher_and_shard_amount(
                ahash::RandomState::new(),
                32,
            )),
            ram_bytes: Arc::new(AtomicUsize::new(0)),
            blocked_count: Arc::new(AtomicU64::new(0)),
            traffic: Arc::new(TrafficCounters::new()),
            total_inserts: Arc::new(AtomicU64::new(0)),
            total_evictions: Arc::new(AtomicU64::new(0)),
            ttl_entries: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Merge subnet-scale counters from a batch flush (O(subnets in batch)).
    /// Windowed: entries older than `window_ns` reset before adding, so a /24
    /// can't accumulate across windows and false-positive the batch blocker.
    pub fn merge_subnet_window(
        &self,
        key: SubnetKey,
        net: IpNetwork,
        events: u32,
        members: Option<&[std::net::IpAddr]>,
        now_ns: u64,
    ) {
        const WINDOW_NS: u64 = SUBNET_WINDOW_NS; // Patch B: one owner — see the const's doc for the 2s->4s rationale.
        // P0 fix: hold the shard lock for the full read-modify-write.
        // The old get().map(|e| e.value().clone()) → mutate → insert pattern
        // dropped the lock between read and write, so two concurrent
        // callers for the same subnet would both see the same baseline,
        // both compute stale deltas, and lose one update.
        self.subnet_table
            .entry(key)
            .and_modify(|rec| {
                // NTP step guard: the clock regressed (wall `now_ns()` is
                // non-monotonic). Never let a stepped-back timestamp set the
                // baseline — elapsed stays 0 (no early reset, no freeze),
                // events still accumulate, and the high-water baseline keeps
                // the window expiring on real forward time.
                let now_ns = now_ns.max(rec.last_updated_ns);
                if now_ns.saturating_sub(rec.last_updated_ns) > WINDOW_NS {
                    rec.total_rps = 0;
                    rec.host_bitmap = [0; 4];
                }
                rec.total_rps = rec.total_rps.saturating_add(events as u64);
                if let Some(ms) = members {
                    for ip in ms {
                        rec.mark_host_v4(*ip);
                    }
                }
                rec.last_updated_ns = now_ns;
            })
            .or_insert_with(|| {
                let mut rec = SubnetRecord {
                    network: net,
                    total_rps: 0,
                    host_bitmap: [0; 4],
                    last_updated_ns: now_ns,
                };
                rec.total_rps = rec.total_rps.saturating_add(events as u64);
                if let Some(ms) = members {
                    for ip in ms {
                        rec.mark_host_v4(*ip);
                    }
                }
                rec
            });
    }

    pub fn reset_subnet_window(&self, key: SubnetKey) {
        if let Some(mut e) = self.subnet_table.get_mut(&key) {
            e.total_rps = 0;
            e.host_bitmap = [0; 4];
        }
    }

    /// CIDR string for a subnet key ("198.51.100.0/24", "2001:db8::/64").
    /// Task 1: single display source for dashboard + batch-block logs.
    /// Empty string for unknown keys (log path only, never gates).
    pub fn subnet_cidr(&self, key: SubnetKey) -> String {
        self.subnet_table
            .get(&key)
            .map_or(String::new(), |e| e.network.to_string())
    }

    /// Exact distinct-host count for a subnet (IPv6 plan Task 2, G1/D1).
    /// Reads the reverse index (`subnet_index`), which every store insert
    /// and eviction already maintains for BOTH families — so v6 /64s, which
    /// no bitmap can cover, get exact cardinality for free.
    /// ponytail ceiling: index membership lives until store eviction, not
    /// the 2s gate window. Acceptable because the second gate leg
    /// (`total_rps`) IS windowed, so a stale swarm can't pass both. Upgrade
    /// path if v6 false positives appear: per-subnet last-seen window.
    pub fn subnet_member_count(&self, key: SubnetKey) -> u64 {
        self.subnet_index
            .get(&key)
            .map_or(0, |ips| ips.len() as u64)
    }

    /// Windowed variant used by the batch gate for v6: counts only members
    /// whose store record was seen within `window_ns` of `now_ns`. The raw
    /// index counts LIFETIME hosts (never pruned on unblock), so without
    /// this a cooled-off /64 with 60 historical members plus a tiny fresh
    /// burst (5 hosts, 100 events) passes the dual gate and batch-blocks
    /// ~55 innocent hosts. O(members) per call — acceptable: gates run on
    /// flush cadence, subnets are small.
    pub fn subnet_member_count_windowed(&self, key: SubnetKey, window_ns: u64, now_ns: u64) -> u64 {
        self.subnet_index.get(&key).map_or(0, |ips| {
            ips.iter()
                .filter(|ip| {
                    self.inner.get(*ip).is_some_and(|v| {
                        let ls = match &v.value().value {
                            Value::IpRecord(rec) => rec.last_seen_ns,
                            _ => 0,
                        };
                        now_ns.saturating_sub(ls) <= window_ns
                    })
                })
                .count() as u64
        })
    }

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
                drop(o);
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
                // Delta applies whether this was a fresh insert or a
                // replacement: `was_blocked` is false for a fresh insert, so
                // the (false -> true) case bumps the count.
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
                drop(o); // release shard lock before touching blocked_set
                let new_size = std::mem::size_of::<Entry>() + std::mem::size_of::<IpAddr>();
                self.apply_growth(key, old_size, new_size, was_blocked, false);
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

    /// Shared growth/shrink bookkeeping for replacements (mirrors the tail
    /// logic of `insert`: signed byte delta + blocked-index transition).
    fn apply_growth(
        &self,
        key: IpAddr,
        old_size: usize,
        new_size: usize,
        was_blocked: bool,
        new_blocked: bool,
    ) {
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
        if !was_blocked && new_blocked {
            self.blocked_count.fetch_add(1, Ordering::Relaxed);
            self.blocked_set.insert(key, ());
        } else if was_blocked && !new_blocked {
            self.blocked_count.fetch_sub(1, Ordering::Relaxed);
            self.blocked_set.remove(&key);
        }
        self.total_inserts.fetch_add(1, Ordering::Relaxed);
    }

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

    /// Update the reverse index for subnet lookups. Call after inserting/updating an IP record.
    pub fn update_subnet_index(
        &self,
        ip_key: IpAddr,
        subnet_key: Option<SubnetKey>,
        is_removal: bool,
    ) {
        let Some(sk) = subnet_key else { return };

        if is_removal {
            // P0 fix: the old path cloned the inner DashSet
            // (DashSet::clone is a DEEP clone, not an Arc handle), removed the
            // IP from the throwaway copy, and emptiness-tested the copy too.
            // The real index entry was NEVER modified — every evicted/unblocked
            // IP leaked into subnet_index forever, and subnet batch-block kept
            // returning dead hosts for subnet batch-block. Removal is now a
            // conditional remove_if on the live entry: removes the IP, drops
            // the subnet key only if the set went empty, and a concurrent
            // insert into the same set makes the predicate false so the key
            // stays. One shard lock per step, no TOCTOU.
            use dashmap::mapref::entry::Entry;
            if let Entry::Occupied(mut e) = self.subnet_index.entry(sk) {
                e.get_mut().remove(&ip_key);
                if e.get().is_empty() {
                    e.remove(); // consumes the entry; shard lock held throughout
                }
            }
        } else {
            self.subnet_index
                .entry(sk)
                .or_insert_with(
                    || std::collections::HashSet::with_hasher(ahash::RandomState::new()),
                )
                .insert(ip_key);
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

#[derive(Debug)]
pub struct StoreStats {
    pub ips_tracked: usize,
    pub blocked: u64,
    pub ram_bytes: usize,
    pub ram_limit_mb: usize,
    pub uptime_secs: u64,
    pub evictions: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Patch B: a pulsed swarm must not reset mid-attack. Two merges 3s
    /// apart (inside the widened window) must accumulate, not zero out.
    #[test]
    fn subnet_window_survives_pulsed_swarm_within_window() {
        let store = Store::new(16);
        let t0 = 1_000_000_000u64;
        let mk = |o: u8| IpAddr::V4(std::net::Ipv4Addr::new(198, 51, 100, o));
        let any = mk(1);
        let net = IpNetwork::of_ip(any);
        let sk = subnet_key_u128(any).unwrap();

        let first: Vec<IpAddr> = (1..=30u8).map(mk).collect();
        store.merge_subnet_window(sk, net, 60, Some(&first), t0);
        // Second pulse 3s later: 30 new hosts, 60 more events.
        let second: Vec<IpAddr> = (31..=60u8).map(mk).collect();
        store.merge_subnet_window(sk, net, 60, Some(&second), t0 + 3_000_000_000);

        let rec = store.subnet_table().get(&sk).unwrap();
        assert_eq!(
            rec.unique_ips(),
            60,
            "host bitmap must accumulate across a 3s pulse gap"
        );
        assert_eq!(
            rec.total_rps, 120,
            "event volume must accumulate across a 3s pulse gap"
        );
    }

    /// Patch B boundary: accumulation must still end past the widened window.
    /// A never-resetting window would let a once-hot /24 block forever.
    #[test]
    fn subnet_window_resets_beyond_widened_boundary() {
        let store = Store::new(16);
        let t0 = 1_000_000_000u64;
        let mk = |o: u8| IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, o));
        let any = mk(1);
        let net = IpNetwork::of_ip(any);
        let sk = subnet_key_u128(any).unwrap();

        let hosts: Vec<IpAddr> = (1..=60u8).map(mk).collect();
        store.merge_subnet_window(sk, net, 480, Some(&hosts), t0);
        // Well past the widened boundary: signal must be gone.
        store.merge_subnet_window(
            sk,
            net,
            3,
            Some(&[mk(200)]),
            t0 + SUBNET_WINDOW_NS + 1_000_000_000,
        );
        let rec = store.subnet_table().get(&sk).unwrap();
        assert_eq!(
            rec.unique_ips(),
            1,
            "stale swarm signal must not survive past the window"
        );
        assert_eq!(rec.total_rps, 3);
    }

    /// NTP backward step: a merge stamped BEFORE the previous one must not
    /// regress the window baseline. The baseline stays at the high-water
    /// mark (events still count — no reset, no loss), and the window still
    /// expires on a later forward timestamp.
    #[test]
    fn subnet_window_clock_step_backwards_keeps_baseline() {
        let store = Store::new(16);
        let t0 = 10_000_000_000u64;
        let mk = |o: u8| IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, o));
        let any = mk(1);
        let net = IpNetwork::of_ip(any);
        let sk = subnet_key_u128(any).unwrap();

        let hosts: Vec<IpAddr> = (1..=30u8).map(mk).collect();
        store.merge_subnet_window(sk, net, 60, Some(&hosts), t0);
        // Clock steps back 4s (≈ one window): the window must NOT reset on
        // this merge (decay = 0, not "window just started"), and the
        // baseline must NOT regress to the stepped timestamp.
        let stepped = t0 - 4_000_000_000;
        store.merge_subnet_window(sk, net, 60, Some(&[mk(200)]), stepped);
        let rec = store.subnet_table().get(&sk).unwrap();
        assert_eq!(
            rec.last_updated_ns, t0,
            "clock step must not regress the baseline"
        );
        assert_eq!(
            rec.total_rps, 120,
            "stepped merge must accumulate, not reset or drop events"
        );
        drop(rec);
        // Forward time must still expire the window.
        store.merge_subnet_window(
            sk,
            net,
            3,
            Some(&[mk(201)]),
            t0 + SUBNET_WINDOW_NS + 1_000_000_000,
        );
        let rec = store.subnet_table().get(&sk).unwrap();
        assert_eq!(
            rec.total_rps, 3,
            "window must expire normally after the step"
        );
    }

    /// Test helper: create an IpRecord with `block_state = Blocked`.
    fn blocked_record(ip: IpAddr) -> IpRecord {
        IpRecord {
            ip,
            request_count: 1,
            ewma_rps: 0.0,
            cusum_s: 0.0,
            baseline_rps: 0.0,
            prev_sample_hot: false,
            sample_count: 0,
            pulse_samples_in_window: 0,
            pulse_window_start_ns: 0,
            first_seen_ns: 0,
            last_seen_ns: 0,
            bytes_in: 0,
            status_dist: [0; 5],
            proto_fingerprint: 0,
            threat_score: 0.0,
            block_state: BlockState::Blocked {
                reason: ramshield_types::BlockReason::HighRps,
                since_ns: 0,
            },
        }
    }

    /// P1 regression: a CapacityExceeded rollback removes the entry from
    /// `inner` but must NOT leave the blocked indexes incremented. Before
    /// the fix, blocked accounting ran before the capacity gate, so every
    /// denied blocked-insert permanently inflated blocked_count and
    /// blocked_set — phantom IPs that `get_all_blocked_ips` (and downstream
    /// unblock-all) would act on.
    /// The decision is the block; members are views of it. A member of an
    /// active CIDR block has no IpRecord of its own (256 records for one
    /// decision is the wrong shape), so a query that reads only per-IP state
    /// lies. This asserts the shared clock: the same `active_cidrs` the
    /// enforcement actor writes is what the query path must consult.
    #[test]
    fn cidr_block_visible_to_ip_query() {
        let store = Store::new(16);
        let net = IpNetwork::new("172.16.30.0".parse().unwrap(), 24).unwrap();
        store.active_cidrs.insert(net, ());

        let member: IpAddr = "172.16.30.44".parse().unwrap();
        // Precondition: no per-IP record exists for the member.
        assert!(store.get(&member).is_none(), "member has no IpRecord");
        // The CIDR clock must answer for it.
        assert_eq!(
            store.is_blocked_by_cidr(&member),
            Some(net),
            "member of active /24 must read as blocked"
        );
        // Outsiders stay clean.
        let outside: IpAddr = "172.16.31.1".parse().unwrap();
        assert!(store.is_blocked_by_cidr(&outside).is_none());
    }

    #[test]
    fn capacity_denial_does_not_pollute_blocked_indexes() {
        let store = Store::new(16);
        let ip: IpAddr = "10.9.9.9".parse().unwrap();
        // ~1 byte budget: any blocked IpRecord insert must be denied.
        let err = store
            .insert(ip, Value::IpRecord(blocked_record(ip)), None, 1)
            .unwrap_err();
        assert!(matches!(err, RsError::CapacityExceeded { .. }));
        assert_eq!(
            store.get_stats().blocked,
            0,
            "blocked_count leaked on rollback"
        );
        assert!(
            store.get_all_blocked_ips().is_empty(),
            "blocked_set leaked on rollback"
        );
        assert!(
            !store.inner().contains_key(&ip),
            "entry itself must be rolled back"
        );

        // Control: a successful blocked insert DOES register in both indexes.
        store
            .insert(
                ip,
                Value::IpRecord(blocked_record(ip)),
                None,
                64 * 1024 * 1024,
            )
            .unwrap();
        assert_eq!(store.get_stats().blocked, 1);
        assert_eq!(store.get_all_blocked_ips(), vec![ip]);
    }

    /// Documented architecture contract, commit item: a fresh insert that
    /// exceeds the budget is rejected BEFORE publication — no phantom entry,
    /// no accounting, no index trace anywhere.
    #[test]
    fn rejected_insert_leaves_no_phantom() {
        let store = Store::new(16);
        let ip: IpAddr = "10.9.9.8".parse().unwrap();
        let denied = store
            .insert(ip, Value::IpRecord(blocked_record(ip)), None, 1)
            .unwrap_err();
        assert!(matches!(denied, RsError::CapacityExceeded { .. }));
        assert!(!store.inner().contains_key(&ip), "no phantom entry");
        assert_eq!(store.ram_bytes(), 0, "no accounting on a rejected insert");
        assert_eq!(
            store.get_stats().blocked,
            0,
            "no blocked index on a rejected insert"
        );
        assert!(store.get_all_blocked_ips().is_empty());
    }

    /// Documented architecture contract, commit item: the capacity gate runs
    /// under the shard lock, so under concurrent fresh inserts the budget is
    /// never silently exceeded and no rejected entry ever persists.
    #[test]
    fn capacity_gate_holds_under_concurrent_fresh_inserts() {
        let store = std::sync::Arc::new(Store::new(16));
        // Budget for ~50 records (blank record + key ~ 128 B): 16 KiB.
        let mut handles = Vec::new();
        for t in 0..8u32 {
            let s = std::sync::Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for i in 0..200u32 {
                    let ip: IpAddr =
                        std::net::Ipv4Addr::new((t % 250) as u8, (i % 250) as u8, 0, 1).into();
                    let _ = s.insert(ip, Value::IpRecord(blank_record(ip)), None, 16 * 1024);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert!(
            store.ram_bytes() <= 16 * 1024,
            "budget silently exceeded: {} bytes",
            store.ram_bytes()
        );
        assert!(store.ram_bytes() > 0, "some inserts must have committed");
        // Every surviving entry must account for real bytes: total equals the
        // sum of the live entries (no phantom residue from rollbacks).
        let sum: usize = store
            .inner()
            .iter()
            .map(|e| {
                std::mem::size_of::<Entry>() + e.value.heap_bytes() + std::mem::size_of::<IpAddr>()
            })
            .sum();
        assert_eq!(
            store.ram_bytes(),
            sum,
            "accounting diverged from live entries"
        );
    }

    #[test]
    fn inline_for_small() {
        let v = Value::from_bytes(&[1u8; 10]);
        assert!(matches!(v, Value::Inline(_)));
    }

    #[test]
    fn blob_for_large() {
        let v = Value::from_bytes(&[1u8; 100]);
        assert!(matches!(v, Value::Blob(_)));
    }

    #[test]
    fn insert_get_remove() {
        let store = Store::new(16);
        store
            .insert(
                "127.0.0.1".parse().unwrap(),
                Value::Counter(1),
                None,
                64 * 1024 * 1024,
            )
            .unwrap();
        assert!(store.get(&"127.0.0.1".parse().unwrap()).is_some());
        store.remove(&"127.0.0.1".parse().unwrap());
        assert!(store.get(&"127.0.0.1".parse().unwrap()).is_none());
    }

    #[test]
    fn ttl_lazy_expiry() {
        let store = Store::new(16);
        store
            .insert(
                "127.0.0.3".parse().unwrap(),
                Value::Counter(1),
                Some(0),
                64 * 1024 * 1024,
            )
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert!(store.get(&"127.0.0.3".parse().unwrap()).is_none());
    }

    /// IPv6 plan Task 1: subnet records must carry family-complete CIDR
    /// metadata. The old `prefix: [u8;3]` (v4-shaped) rendered v6 /64s as
    /// garbage three-octet strings in the dashboard and batch-block logs.
    #[test]
    fn subnet_record_cidr_display_both_families() {
        let store = Store::new(8);
        let v4: IpAddr = "198.51.100.7".parse().unwrap();
        let v6: IpAddr = "2001:db8:abcd::5".parse().unwrap();
        let (k4, n4) = subnet::subnet_key(v4).unwrap();
        let (k6, n6) = subnet::subnet_key(v6).unwrap();
        store.merge_subnet_window(k4, n4, 5, Some(&[v4]), 0);
        store.merge_subnet_window(k6, n6, 5, Some(&[v6]), 0);
        assert_eq!(store.subnet_cidr(k4), "198.51.100.0/24");
        assert_eq!(store.subnet_cidr(k6), "2001:db8:abcd::/64");
    }

    #[test]
    fn subnet_window_v6_key() {
        let store = Store::new(16);
        let v6: IpAddr = "2001:db8::1".parse().unwrap();
        let key = subnet::subnet_key_u128(v6).unwrap();
        let net = IpNetwork::of_ip(v6);
        store.merge_subnet_window(key, net, 5, Some(&[v6]), 1_000_000_000);
        assert_eq!(store.subnet_table().get(&key).unwrap().total_rps, 5);
        store.reset_subnet_window(key);
        assert_eq!(store.subnet_table().get(&key).unwrap().total_rps, 0);
    }

    /// P0 regression: capacity check used to be racy — two threads both
    /// reading pre-insert `ram_bytes` could both decide "fits" and both
    /// insert, silently exceeding the budget. The CAS-based reservation
    /// ensures only one thread succeeds; the other rolls back.
    #[test]
    fn capacity_race_serializes_via_cas() {
        use std::sync::Arc;
        use std::thread;
        let store = Arc::new(Store::new(16));
        // Tight budget: 1 KiB net-new total.
        let limit = 1024;
        let mut handles = vec![];
        // 64 distinct IPs, each with a 64-byte payload (size_of::<Entry> +
        // 64 bytes value). 64 × ~200B = ~13 KiB > 1 KiB limit.
        // Some inserts must fail with CapacityExceeded; the survivors must
        // leave ram_bytes ≤ limit.
        for n in 0..64u8 {
            let s = store.clone();
            handles.push(thread::spawn(move || {
                let ip: IpAddr = format!("10.1.1.{}", n).parse().unwrap();
                s.insert(ip, Value::Inline(vec![0u8; 64]), None, limit)
            }));
        }
        let mut ok = 0;
        let mut denied = 0;
        for h in handles {
            match h.join().unwrap() {
                Ok(_) => ok += 1,
                Err(RsError::CapacityExceeded { .. }) => denied += 1,
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        assert!(denied > 0, "some inserts must be denied at 1KiB / 64 IPs");
        assert_eq!(ok + denied, 64);
        // Critical: ram_bytes must not exceed the limit. The old racy code
        // could let the budget be silently blown here.
        let used = store.ram_bytes();
        assert!(
            used <= limit,
            "ram_bytes {} must not exceed limit {} after race",
            used,
            limit
        );
    }

    /// P0 regression: subnet_index must be cleaned on eviction/removal.
    /// Without this, every churned IP leaves a phantom in its subnet's
    /// DashSet, growing the index unboundedly.
    #[test]
    fn subnet_index_cleaned_on_evict_and_remove() {
        let store = Store::new(16);
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        let sk = subnet_key_u128(ip).unwrap();
        let net = IpNetwork::of_ip(ip);
        // Build a real record in `inner` AND populate subnet_index — this
        // is the steady-state shape detection creates.
        store
            .insert(
                ip,
                Value::IpRecord(crate::IpRecord {
                    ip,
                    request_count: 0,
                    ewma_rps: 0.0,
                    cusum_s: 0.0,
                    baseline_rps: 0.0,
                    prev_sample_hot: false,
                    sample_count: 0,
                    pulse_samples_in_window: 0,
                    pulse_window_start_ns: 0,
                    first_seen_ns: 0,
                    last_seen_ns: 0,
                    bytes_in: 0,
                    status_dist: [0; 5],
                    proto_fingerprint: 0,
                    threat_score: 0.0,
                    block_state: crate::BlockState::Clean,
                }),
                Some(0),
                1 << 20,
            )
            .unwrap();
        store.update_subnet_index(ip, Some(sk), false);
        // Also seed the subnet window so the same subnet is recognized.
        store.merge_subnet_window(sk, net, 1, Some(&[ip]), 1_000_000_000);
        assert_eq!(store.subnet_member_count(sk), 1);

        // evict_batch: TTL already expired by the time we call.
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.evict_batch(&[ip]);
        assert!(
            store.subnet_member_count(sk) == 0,
            "subnet_index must be empty after evict_batch; stale entries leak memory"
        );
    }

    /// P0 regression: Store::remove must also clean subnet_index, or any
    /// caller that removes an IP directly (e.g. dashboard unblock endpoint)
    /// leaves a phantom in the reverse index.
    #[test]
    fn subnet_index_cleaned_on_remove() {
        let store = Store::new(16);
        let ip: IpAddr = "10.0.0.2".parse().unwrap();
        let sk = subnet_key_u128(ip).unwrap();
        store.insert(ip, Value::Counter(1), None, 1 << 20).unwrap();
        store.update_subnet_index(ip, Some(sk), false);
        assert_eq!(store.subnet_member_count(sk), 1);
        store.remove(&ip);
        assert!(
            store.subnet_member_count(sk) == 0,
            "subnet_index must be empty after remove; stale entries leak memory"
        );
    }

    /// P0 regression: get_all_blocked_ips must be O(blocked) not O(store).
    /// Old code did `self.inner.iter().filter(|e| e.value().is_blocked())`
    /// which walks every IP in the store. The fix maintains a `blocked_set`
    /// DashSet updated on BlockState transitions, making the lookup O(B).
    /// This test plants 1000 clean IPs and 5 blocked, asserts the function
    /// returns exactly the 5 blocked IPs.
    #[test]
    fn get_all_blocked_ips_uses_index() {
        let store = Store::new(16);
        // 1000 clean IPs
        for n in 0..1000u16 {
            let ip: IpAddr = format!("10.5.{}.{}", n / 256, n % 256).parse().unwrap();
            store
                .insert(ip, Value::Counter(1), None, 64 * 1024 * 1024)
                .unwrap();
        }
        // 5 blocked IPs
        let blocked_ips: Vec<IpAddr> = (0..5u16)
            .map(|n| format!("10.99.0.{}", n).parse().unwrap())
            .collect();
        for ip in &blocked_ips {
            store
                .insert(
                    *ip,
                    Value::IpRecord(blocked_record(*ip)),
                    None,
                    64 * 1024 * 1024,
                )
                .unwrap();
        }
        let got: std::collections::HashSet<IpAddr> =
            store.get_all_blocked_ips().into_iter().collect();
        let want: std::collections::HashSet<IpAddr> = blocked_ips.into_iter().collect();
        assert_eq!(
            got, want,
            "get_all_blocked_ips must return exactly the blocked set"
        );
    }

    /// P0 regression: get_all_blocked_ips must reflect unblock transitions.
    /// Insert blocked, then replace with clean — set should remove the IP.
    #[test]
    fn get_all_blocked_ips_tracks_unblock() {
        let store = Store::new(16);
        let ip: IpAddr = "10.6.6.6".parse().unwrap();
        store
            .insert(
                ip,
                Value::IpRecord(blocked_record(ip)),
                None,
                64 * 1024 * 1024,
            )
            .unwrap();
        assert_eq!(store.get_all_blocked_ips(), vec![ip]);
        // Replace with clean
        store
            .insert(ip, Value::Counter(1), None, 64 * 1024 * 1024)
            .unwrap();
        assert!(store.get_all_blocked_ips().is_empty());
    }

    /// P0 regression: blocked_set must stay in sync across all 4 mutation paths.
    /// Insert, replace (clean→blocked), replace (blocked→clean), remove.
    #[test]
    fn blocked_set_consistent_across_mutations() {
        let store = Store::new(16);
        let ip: IpAddr = "10.7.7.7".parse().unwrap();
        // 1. Insert clean — set must NOT contain ip.
        store
            .insert(ip, Value::Counter(1), None, 64 * 1024 * 1024)
            .unwrap();
        assert!(store.get_all_blocked_ips().is_empty());
        // 2. Replace clean→blocked — set must contain ip.
        store
            .insert(
                ip,
                Value::IpRecord(blocked_record(ip)),
                None,
                64 * 1024 * 1024,
            )
            .unwrap();
        assert_eq!(store.get_all_blocked_ips(), vec![ip]);
        // 3. Replace blocked→clean — set must NOT contain ip.
        store
            .insert(ip, Value::Counter(1), None, 64 * 1024 * 1024)
            .unwrap();
        assert!(store.get_all_blocked_ips().is_empty());
        // 4. Re-block, then remove — set must NOT contain ip.
        store
            .insert(
                ip,
                Value::IpRecord(blocked_record(ip)),
                None,
                64 * 1024 * 1024,
            )
            .unwrap();
        assert_eq!(store.get_all_blocked_ips(), vec![ip]);
        store.remove(&ip);
        assert!(store.get_all_blocked_ips().is_empty());
    }

    /// P0 regression: blocked_set must stay correct under concurrent transitions.
    /// 8 threads racing to insert (clean or blocked) the same IP — final set
    /// state must match the actual `is_blocked()` state of the entry.
    #[test]
    fn blocked_set_concurrent_transitions_converge() {
        use std::sync::Arc;
        use std::thread;
        let store = Arc::new(Store::new(16));
        let ip: IpAddr = "10.8.8.8".parse().unwrap();
        let mut handles = vec![];
        for n in 0..8u32 {
            let s = store.clone();
            handles.push(thread::spawn(move || {
                for i in 0..100u32 {
                    if (n + i) % 2 == 0 {
                        s.insert(ip, Value::Counter(1), None, 64 * 1024 * 1024)
                            .unwrap();
                    } else {
                        s.insert(
                            ip,
                            Value::IpRecord(blocked_record(ip)),
                            None,
                            64 * 1024 * 1024,
                        )
                        .unwrap();
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // Final state of the entry decides whether the set contains ip.
        let entry_blocked = store.get(&ip).map(|v| v.is_blocked()).unwrap_or(false);
        let set_contains = !store.get_all_blocked_ips().is_empty();
        assert_eq!(
            entry_blocked, set_contains,
            "blocked_set membership must match the entry's is_blocked() state"
        );
    }

    /// P0 regression: merge_subnet_window read-modify-write must not lose
    /// updates under concurrency. The old code did get().clone() → mutate
    /// → insert, dropping the shard lock between read and write, so two
    /// concurrent calls for the same subnet would both see the same
    /// baseline and one update would be lost.
    #[test]
    fn merge_subnet_window_concurrent_no_lost_updates() {
        use std::sync::Arc;
        use std::thread;
        let store = Arc::new(Store::new(16));
        let sk: SubnetKey = 0x0a14_1e00_0000_0000_0000_0000_0000_0000;
        let net = IpNetwork::ipv4_subnet(std::net::Ipv4Addr::new(10, 20, 30, 0));
        // 16 threads each call merge_subnet_window with 1 event for the
        // same subnet, 100 ms apart so the window doesn't roll over.
        // Total expected: 16 events; old code frequently observed <16.
        let mut handles = vec![];
        for _ in 0..16 {
            let s = store.clone();
            handles.push(thread::spawn(move || {
                s.merge_subnet_window(sk, net, 1, None, 1_000_000_000);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let total = store.subnet_table().get(&sk).unwrap().total_rps;
        assert_eq!(total, 16, "all 16 concurrent merges must be counted");
    }

    /// P1-6 regression: evict_expired must remove TTL-expired entries and
    /// reclaim ram_bytes. Insert with TTL=1s, sleep, sweep, assert gone.
    #[test]
    fn evict_expired_removes_ttl_entries() {
        use std::net::Ipv4Addr;
        let store = Store::new(16);
        let ip: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 99, 0, 1));
        let ram_lim = 64 * 1024 * 1024;
        store
            .insert(ip, Value::Counter(42), Some(1), ram_lim)
            .unwrap();
        assert_eq!(store.len(), 1, "entry present before expiry");
        assert!(store.ram_bytes() > 0, "ram_bytes non-zero after insert");
        std::thread::sleep(std::time::Duration::from_secs(2));
        let evicted = store.evict_expired();
        assert_eq!(evicted, 1, "evict_expired must remove the expired entry");
        assert_eq!(store.len(), 0, "store empty after eviction");
        assert_eq!(store.ram_bytes(), 0, "ram_bytes zeroed after eviction");
    }

    /// IPv6 plan Task 5: the v6 gate leg (Task 2) reads `subnet_index`
    /// cardinality — stale members would count as ghosts forever if any
    /// eviction path skipped index cleanup. v6 must behave exactly like v4.
    #[test]
    fn v6_eviction_cleans_subnet_index() {
        let store = Store::new(16);
        let ram_lim = 64 * 1024 * 1024;
        let hosts: Vec<IpAddr> = (1..=3u16)
            .map(|o| IpAddr::V6(std::net::Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, o)))
            .collect();
        let sk = subnet_key_u128(hosts[0]).unwrap();
        for h in &hosts {
            store
                .insert(*h, Value::Counter(1), Some(1), ram_lim)
                .unwrap();
            store.update_subnet_index(*h, Some(sk), false);
        }
        assert_eq!(store.subnet_member_count(sk), 3, "index seeded");
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert_eq!(store.evict_expired(), 3);
        assert_eq!(
            store.subnet_member_count(sk),
            0,
            "evicted v6 hosts must leave zero gate-countable ghosts"
        );
    }

    /// P0 regression (round-4 Q3): `update_ip` mutating a live Blocked record
    /// must NOT revert block_state — the merge_record get/insert race let a
    /// stale Clean snapshot resurrect blocked attackers.
    #[test]
    fn update_ip_preserves_block_state() {
        use std::net::Ipv4Addr;
        let store = Store::new(16);
        let ip: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 98, 0, 1));
        let ram_lim = 64 * 1024 * 1024;
        store
            .insert(ip, Value::IpRecord(blocked_record(ip)), None, ram_lim)
            .unwrap();
        let (n, stored) = store.update_ip(ip, blank_record(ip), ram_lim, |r| {
            r.request_count += 5;
            r.request_count
        });
        assert!(stored);
        assert_eq!(n, 6);
        match store.get(&ip) {
            Some(Value::IpRecord(r)) => {
                assert!(
                    matches!(r.block_state, BlockState::Blocked { .. }),
                    "update_ip clobbered block state"
                );
                assert_eq!(r.request_count, 6);
            }
            other => panic!("expected IpRecord, got {other:?}"),
        }
    }

    /// Vacant path: creates the record, bumps accounting; capacity-exceeded
    /// path must refuse (stored=false) and leave ram_bytes untouched.
    #[test]
    fn update_ip_vacant_and_capacity() {
        use std::net::Ipv4Addr;
        let store = Store::new(16);
        let ip: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 97, 0, 1));
        let big = 64 * 1024 * 1024;
        let (v, stored) = store.update_ip(ip, blank_record(ip), big, |r| {
            r.request_count = 42;
            r.request_count
        });
        assert!(stored);
        assert_eq!(v, 42);
        assert_eq!(store.len(), 1);
        assert!(store.ram_bytes() > 0);
        let ram_before = store.ram_bytes();
        // Tiny budget: net-new key refused, nothing created, no leak.
        let ip2: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 97, 0, 2));
        let (_, stored2) = store.update_ip(ip2, blank_record(ip2), 1, |_r| ());
        assert!(!stored2, "must refuse net-new when budget exhausted");
        assert_eq!(store.ram_bytes(), ram_before, "refused insert leaked bytes");
        assert_eq!(store.len(), 1);
    }

    fn blank_record(ip: IpAddr) -> IpRecord {
        IpRecord {
            ip,
            request_count: 0,
            ewma_rps: 0.0,
            cusum_s: 0.0,
            baseline_rps: 0.0,
            prev_sample_hot: false,
            sample_count: 0,
            pulse_samples_in_window: 0,
            pulse_window_start_ns: 0,
            first_seen_ns: 0,
            last_seen_ns: 0,
            bytes_in: 0,
            status_dist: [0; 5],
            proto_fingerprint: 0,
            threat_score: 0.0,
            block_state: BlockState::Clean,
        }
    }

    #[test]
    fn threat_sample_queue_is_bounded() {
        // P1-9: the queue must not grow unbounded when the forecaster (its
        // only drainer) is stalled or disabled.
        let s = TrafficCounters::new();
        let ip: IpAddr = "10.98.0.1".parse().unwrap();
        let many: Vec<(IpAddr, f32)> = (0..3000u32).map(|i| (ip, i as f32 + 0.5)).collect();
        s.push_threat_samples(many);
        assert!(
            s.threat_sample.len() <= 1024,
            "queue exceeded bound: {}",
            s.threat_sample.len()
        );
    }
}
