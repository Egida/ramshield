//! Unified RamShield storage: sharded in-memory Store (from src, perf-tuned)
//! + WAL / BlobStore durability modules (from crate).
//!
//! Types: `Value::IpRecord` is the canonical per-IP entry; subnet keys are
//! `u128` (IPv4 packed low-32, IPv6 full address) with `IpNetwork` metadata.

pub mod checkpoint_shared;
pub mod subnet;
pub mod wal;

pub use subnet::{subnet_key_u128, subnet_key_v4, subnet_key_v6};

mod record;
mod store;

pub use record::{BlockState, Entry, IpRecord, SubnetRecord, TrafficCounters, Value};

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
mod tests;
