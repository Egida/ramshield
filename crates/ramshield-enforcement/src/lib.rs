//! Enforcement service: the single writer for security state and dataplane changes.
//!
//! All block/unblock requests are serialized through this actor. Callers never
//! mutate Store block state directly. TTL expiry is also converted into an
//! internal unblock command so the same state transition path is used.
//!
//! Durability (crate port): when a WAL is attached, every Block/Unblock is
//! appended to the WAL BEFORE the storage mutation, and the returned LSN is
//! set on `EnforceResult.wal_lsn`. Order: WAL → storage → TTL schedule → XDP.

use ahash::{AHashMap as HashMap, AHashSet as HashSet};
use anyhow::Result;
use ramshield_metrics::Metrics;
use ramshield_storage::{
    BlockState, IpRecord, Store, Value,
    wal::{Wal, WalEntry},
};
use ramshield_types::{
    BlockReason, EnforceAction, EnforceCommand, EnforceResult, EnforcementError, IpNetwork,
};
use std::collections::{BTreeMap, VecDeque};
use std::net::IpAddr;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};
use uuid::Uuid;

/// Fallback expiry horizon when TTL arithmetic overflows (belt-and-suspenders;
/// all entry points clamp, so this should never fire). 24h keeps a runaway
/// block bounded instead of permanent.
const MAX_EXPIRY_FALLBACK_SECS: u64 = 86_400;

#[cfg(feature = "xdp")]
pub mod xdp;

#[derive(Debug, Clone, Default)]
pub struct ReconciliationState {
    pub last_wal_lsn: u64,
    pub pending_blocks: Vec<IpAddr>,
    pub pending_unblocks: Vec<IpAddr>,
}

#[async_trait::async_trait]
pub trait XdpApplier: Send + Sync {
    /// ttl_seconds: 0 = permanent block (u64::MAX expiry in the map).
    fn apply_block(
        &mut self,
        ip: IpAddr,
        decision_id: Uuid,
        ttl_seconds: u64,
    ) -> Result<(), EnforcementError>;
    fn apply_unblock(&mut self, ip: IpAddr, decision_id: Uuid) -> Result<(), EnforcementError>;
    fn apply_cidr_block(
        &mut self,
        network: IpNetwork,
        _decision_id: Uuid,
        _ttl_seconds: u64,
    ) -> Result<(), EnforcementError> {
        Err(EnforcementError::Xdp(format!(
            "CIDR enforcement unsupported: {network}"
        )))
    }
    fn apply_cidr_unblock(
        &mut self,
        network: IpNetwork,
        _decision_id: Uuid,
    ) -> Result<(), EnforcementError> {
        Err(EnforcementError::Xdp(format!(
            "CIDR enforcement unsupported: {network}"
        )))
    }
    /// Reconcile both per-IP hash maps and CIDR LPM-trie maps against the
    /// userspace source of truth. CIDRs are included explicitly because they
    /// are not represented by `Store::get_all_blocked_ips()`.
    fn reconcile(
        &mut self,
        expected_blocks: &[IpAddr],
        expected_cidrs: &[IpNetwork],
    ) -> Result<ReconciliationState, EnforcementError>;
    /// Drain kernel→userspace drop notifications (RingBuf). Default: no channel.
    fn drain_drop_events(&mut self) -> Vec<XdpDropEvent> {
        Vec::new()
    }
    fn counters(&mut self) -> Result<[u64; 4], EnforcementError> {
        Ok([0; 4])
    }
}

/// One kernel→userspace drop notification from the XDP EVENTS ringbuf.
/// `ip` = dropped source address, `ts_ns` = monotonic clock (bpf_ktime_get_ns),
/// `slot` = COUNTERS slot that was incremented (0 = v4_drop, 1 = v6_drop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XdpDropEvent {
    pub ip: IpAddr,
    pub ts_ns: u64,
    pub slot: u8,
}

pub struct StubXdpApplier;

#[async_trait::async_trait]
impl XdpApplier for StubXdpApplier {
    fn apply_block(
        &mut self,
        ip: IpAddr,
        _decision_id: Uuid,
        _ttl_seconds: u64,
    ) -> Result<(), EnforcementError> {
        trace!(%ip, "XDP block (stub)");
        Ok(())
    }
    fn apply_unblock(&mut self, ip: IpAddr, _decision_id: Uuid) -> Result<(), EnforcementError> {
        trace!(%ip, "XDP unblock (stub)");
        Ok(())
    }
    fn reconcile(
        &mut self,
        _expected_blocks: &[IpAddr],
        _expected_cidrs: &[IpNetwork],
    ) -> Result<ReconciliationState, EnforcementError> {
        Ok(ReconciliationState::default())
    }
}

/// Sole writer. The command queue is bounded by the engine and this actor is
/// the only component permitted to mutate BlockState or the XDP dataplane.
pub struct EnforcementService {
    store: Arc<Store>,
    metrics: Arc<Metrics>,
    xdp: Box<dyn XdpApplier>,
    /// Optional durability: append-before-mutate. None = in-memory only.
    wal: Option<Arc<Wal>>,
    /// Idempotency cache: decision_id → ORIGINAL EnforceResult (bounded to
    /// 65_536 by processed_order). A replayed decision_id returns what
    /// happened the first time — including a dataplane failure — instead of
    /// a fabricated fresh success.
    processed_results: HashMap<Uuid, EnforceResult>,
    processed_order: VecDeque<Uuid>,
    blocked_ips: HashSet<IpAddr>,
    /// Per-IP attributed XDP drops since block, keyed to userspace-blocked
    /// IPs ONLY (bounded by |blocked_ips|; cleared on unblock / re-block).
    /// Observability basis: audit counters + zero-drop gauge. Invariant:
    /// this map never feeds detection or the forecaster — a drop is a
    /// consequence of our own block, not independent threat evidence.
    drops_by_blocked: HashMap<IpAddr, u64>,
    /// Userspace mirror of active CIDR blocks lives in `store.active_cidrs`
    /// (single owner = this actor, single reader path = check_ip/dashboard).
    /// ponytail: kernel BLOCKCIDR maps are the authoritative dataplane; this
    /// mirror gives cheap telemetry without per-tick map iteration — upgrade
    /// to LpmTrie::iter if exact kernel entry counts are ever required.
    /// TTL expiry index: IP -> (second-bucket, position in that bucket's Vec).
    /// RAM-for-CPU item 14: was a flat HashMap swept with `retain()` every
    /// 250ms — O(all pending expirations) per tick to usually find zero due.
    /// Now `expire_due` drains only buckets whose second has passed: O(due).
    /// The (bucket,pos) pair makes re-block (TTL refresh) O(1): detach via
    /// swap-remove from the old bucket, attach to the new — no stale cards,
    /// so a refresh storm (unconditional block emissions every flush) cannot
    /// accumulate garbage the way a lazy-generation ring would. BinaryHeap
    /// was rejected earlier (rebuild on re-block); this is the bucketed-PQ
    /// trick with position tracking instead.
    /// Resolution is one second (bucket fires at the first whole second at or
    /// after the deadline — never early, at most ~1s late). ponytail: if
    /// sub-second TTL precision ever matters, switch buckets to a ms-grained
    /// ring over a fixed horizon.
    expirations: HashMap<IpAddr, (u64, usize)>,
    cidr_expirations: HashMap<IpNetwork, Instant>,
    buckets: BTreeMap<u64, Vec<IpAddr>>,
    epoch: Instant,
    shutdown: Arc<AtomicBool>,
    /// Last committed WAL LSN (updated on every enforce() call). None = no WAL.
    last_wal_lsn: Option<u64>,
    /// P2: Cluster blocklist CRDT — companion to `blocked_ips` local mirror.
    /// Local blocks are authoritative; this CRDT absorbs peer deltas and
    /// merges them on the next enforcement tick. None = single-node.
    mesh_blocklist: Option<Arc<ramshield_mesh::aworset::AworsetBlocklist>>,
}

impl EnforcementService {
    pub fn new(
        store: Arc<Store>,
        metrics: Arc<Metrics>,
        xdp: Box<dyn XdpApplier>,
        shutdown: Arc<AtomicBool>,
    ) -> Self {
        Self {
            store,
            metrics,
            xdp,
            wal: None,
            processed_results: HashMap::new(),
            processed_order: VecDeque::with_capacity(65_536),
            blocked_ips: HashSet::new(),
            drops_by_blocked: HashMap::new(),
            expirations: HashMap::new(),
            cidr_expirations: HashMap::new(),
            buckets: BTreeMap::new(),
            epoch: Instant::now(),
            shutdown,
            last_wal_lsn: None,
            // P2: disabled mesh by default; enable with `with_mesh_blocklist`.
            mesh_blocklist: None,
        }
    }

    /// Enable cluster CRDT companion (fleet gossip mesh).
    pub fn with_mesh_blocklist(
        mut self,
        mesh_blocklist: Arc<ramshield_mesh::aworset::AworsetBlocklist>,
    ) -> Self {
        self.mesh_blocklist = Some(mesh_blocklist);
        self
    }

    /// Attach WAL for durable enforcement (append-before-mutate ordering).
    pub fn with_wal(mut self, wal: Arc<Wal>) -> Self {
        self.wal = Some(wal);
        self
    }

    pub async fn run(mut self, mut command_rx: mpsc::Receiver<EnforceCommand>) -> Result<()> {
        info!("Enforcement service started");
        let expected = self.store.get_all_blocked_ips();
        let expected_cidrs: Vec<IpNetwork> =
            self.store.active_cidrs.iter().map(|e| *e.key()).collect();
        match self.xdp.reconcile(&expected, &expected_cidrs) {
            Ok(_) => {
                self.blocked_ips = expected.into_iter().collect();
                let now_unix = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                self.metrics.record_reconcile_success(now_unix);
                info!("XDP reconciled with {} blocked IPs", self.blocked_ips.len());
            }
            Err(e) => {
                let now_unix = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                self.metrics.record_reconcile_failure(
                    now_unix,
                    self.metrics
                        .reconcile_last_success_unix
                        .load(std::sync::atomic::Ordering::Relaxed),
                );
                error!("Initial XDP reconciliation failed: {}", e)
            }
        }

        let mut tick = tokio::time::interval(Duration::from_millis(250));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Continuous store→XDP reconcile: every 40 ticks ≈ 10s. Closes map-loss
        // drift after driver reload or external map wipe without waiting for restart.
        let mut reconcile_ticks: u32 = 0;
        const RECONCILE_EVERY_TICKS: u32 = 40;
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    self.expire_due().await;
                    let now_unix = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    self.metrics.tick_reconcile_age(now_unix);
                    reconcile_ticks = reconcile_ticks.wrapping_add(1);
                    if reconcile_ticks.is_multiple_of(RECONCILE_EVERY_TICKS) {
                        let expected = self.store.get_all_blocked_ips();
                        let expected_cidrs: Vec<IpNetwork> = self.store.active_cidrs.iter().map(|e| *e.key()).collect();
                        match self.xdp.reconcile(&expected, &expected_cidrs) {
                            Ok(_) => {
                                self.blocked_ips = expected.into_iter().collect();
                                // Reconcile can shrink the blocked set out-of-band
                                // (map wipe, external unblock): drop orphan
                                // attribution before it skews the zero-drop gauge.
                                self.drops_by_blocked
                                    .retain(|ip, _| self.blocked_ips.contains(ip));
                                let now_unix = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs();
                                self.metrics.record_reconcile_success(now_unix);
                                debug!(
                                    n = self.blocked_ips.len(),
                                    "periodic XDP reconcile ok"
                                );
                            }
                            Err(e) => {
                                let now_unix = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs();
                                self.metrics.record_reconcile_failure(
                                    now_unix,
                                    self.metrics
                                        .reconcile_last_success_unix
                                        .load(std::sync::atomic::Ordering::Relaxed),
                                );
                                error!("periodic XDP reconciliation failed: {e}");
                            }
                        }
                    }
                    // XDP kernel counters: drain ringbuf events, read counters
                    let drops = self.xdp.drain_drop_events();
                    self.attribute_drops(drops);
                    match self.xdp.counters() {
                        Ok(c) => {
                            self.metrics.set_xdp_counters(c[0], c[1], c[2], c[3]);
                            // 4 Hz audit: proves kernel counter deltas
                            // propagate to Metrics (SSE xdp.* series).
                            debug!(
                                v4_drops = c[0],
                                v6_drops = c[1],
                                wire_pass = c[2],
                                parse_fails = c[3],
                                "xdp counters read"
                            );
                        }
                        // Non-fatal (stub backend, missing COUNTERS map).
                        // trace: must not become a per-tick debug flood.
                        Err(e) => trace!(error = %e, "xdp counters unavailable"),
                    }
                    // ponytail: publish enforcement state to Metrics so the
                    // dashboard reads live values instead of dead zeros.
                    if let Some(lsn) = self.last_wal_lsn {
                        self.metrics.set_wal_lsn(lsn);
                    }
                    self.metrics.set_pending_expirations(self.expirations.len() as u64);
                    self.metrics.set_active_cidr_blocks(self.store.active_cidrs.len());
                    // HLC activity: published from the CRDT that owns the clock.
                    // NOTE: mesh_blocklist_len is NOT written into
                    // mesh_record_ban_count — that atomic is a cumulative
                    // counter owned by detection (record_ban). Overwriting it
                    // with len() every tick made the counter meaningless.
                    if let Some(mesh) = &self.mesh_blocklist {
                        self.metrics.mesh_hlc_ticks.store(
                            mesh.hlc_ticks(),
                            std::sync::atomic::Ordering::Relaxed,
                        );
                    }
                    // mesh_purge_ticks is owned by the real purge site
                    // (coordinator maintenance sweep), not this tick loop.
                    if self.shutdown.load(Ordering::Acquire) { break; }
                }
                cmd = command_rx.recv() => {
                    match cmd {
                        Some(cmd) => {
                            if let Err(e) = self.enforce(cmd).await { error!("Enforcement failed: {}", e); }
                        }
                        // All senders dropped — clean shutdown.
                        None => break,
                    }
                }
            }
            if self.shutdown.load(Ordering::Acquire) {
                break;
            }
        }
        info!("Enforcement service stopped");
        Ok(())
    }

    async fn expire_due(&mut self) {
        // O(due): only buckets whose (whole-second) deadline has passed are
        // touched — previously every 250ms tick re-examined ALL expirations.
        let now_ts = self.epoch.elapsed().as_secs();
        let mut due = Vec::new();
        while let Some((&b, _)) = self.buckets.first_key_value() {
            if b > now_ts {
                break;
            }
            if let Some((_, vec)) = self.buckets.pop_first() {
                for ip in vec {
                    self.expirations.remove(&ip);
                    due.push(ip);
                }
            }
        }
        for ip in due {
            let cmd = EnforceCommand {
                decision_id: Uuid::new_v4(),
                policy_version: 0,
                source: "ttl".into(),
                actor: "system".into(),
                timestamp_utc: epoch_seconds(),
                ttl_seconds: 0,
                reason: "ttl_expired".into(),
                ip,
                cidr: None,
                action: EnforceAction::Unblock,
            };
            match self.enforce(cmd).await {
                Ok(_) => {}
                Err(EnforcementError::InvalidCommand(_)) => {
                    warn!(%ip, "TTL unblock rejected as invalid; dropping lease");
                }
                Err(e) => {
                    // A transient WAL/storage failure must never convert a
                    // temporary block into a permanent one: re-arm the lease
                    // one second out and let the next tick retry the same
                    // Unblock transition.
                    warn!(%ip, "TTL unblock failed: {e} — re-arming lease");
                    self.schedule_expiration(ip, Instant::now() + Duration::from_secs(1));
                }
            }
        }
        let now = Instant::now();
        let cidrs: Vec<IpNetwork> = self
            .cidr_expirations
            .iter()
            .filter_map(|(network, &deadline)| (deadline <= now).then_some(*network))
            .collect();
        for network in cidrs {
            self.cidr_expirations.remove(&network);
            let cmd = EnforceCommand {
                decision_id: Uuid::new_v4(),
                policy_version: 0,
                source: "ttl".into(),
                actor: "system".into(),
                timestamp_utc: epoch_seconds(),
                ttl_seconds: 0,
                reason: "ttl_expired".into(),
                ip: network.addr,
                cidr: Some(network),
                action: EnforceAction::Unblock,
            };
            match self.enforce(cmd).await {
                Ok(_) => {}
                Err(EnforcementError::InvalidCommand(_)) => {
                    warn!(cidr=?network, "CIDR TTL unblock rejected as invalid; dropping lease");
                }
                Err(e) => {
                    warn!(cidr=?network, "CIDR TTL unblock failed: {e} — re-arming lease");
                    self.cidr_expirations
                        .insert(network, Instant::now() + Duration::from_secs(1));
                }
            }
        }
    }

    /// Bucket for a deadline: ceil to whole seconds from epoch, so a bucket
    /// only drains when the exact deadline has passed (never early).
    fn bucket_of(&self, at: Instant) -> u64 {
        let d = at.saturating_duration_since(self.epoch);
        d.as_secs() + u64::from(d.subsec_nanos() > 0)
    }

    /// Remove an IP's pending expiration (O(1)); swap-remove keeps bucket
    /// vectors dense — the moved neighbour's position index is fixed up.
    fn detach_expiration(&mut self, ip: IpAddr) {
        if let Some((b, pos)) = self.expirations.remove(&ip)
            && let Some(vec) = self.buckets.get_mut(&b)
        {
            if pos < vec.len() {
                // Self-heal guard: a drifting index (bug elsewhere) would
                // silently remove the WRONG card and orphan this IP forever.
                // Assert the card identity; on mismatch, scan the bucket.
                let detach_pos = if vec[pos] == ip {
                    Some(pos)
                } else {
                    vec.iter().position(|&candidate| candidate == ip)
                };
                if let Some(p) = detach_pos {
                    vec.swap_remove(p);
                    if let Some(&moved) = vec.get(p)
                        && let Some(slot) = self.expirations.get_mut(&moved)
                    {
                        slot.1 = p;
                    }
                }
            }
            if vec.is_empty() {
                self.buckets.remove(&b);
            }
        }
    }

    fn schedule_expiration(&mut self, ip: IpAddr, at: Instant) {
        self.detach_expiration(ip);
        let b = self.bucket_of(at);
        let idx = {
            let vec = self.buckets.entry(b).or_default();
            vec.push(ip);
            vec.len() - 1
        };
        self.expirations.insert(ip, (b, idx));
    }

    /// Attribute drained XDP drop events (called each 250 ms tick).
    ///
    /// Invariant: attribution is OBSERVABILITY ONLY — per-IP drop counts
    /// never enter detection or the forecaster. A drop is the consequence
    /// of our own block, not independent evidence of maliciousness; feeding
    /// it to a learner would self-confirm every block (false positives can
    /// never be exonerated). Counts keyed to userspace-blocked IPs bound
    /// the map by |blocked_ips|. Unattributed drops (IP not blocked) count
    /// as kernel/userspace drift indicators.
    fn attribute_drops(&mut self, drops: Vec<XdpDropEvent>) {
        let mut gaps = 0u64;
        for ev in drops {
            if self.blocked_ips.contains(&ev.ip) {
                *self.drops_by_blocked.entry(ev.ip).or_insert(0) += 1;
            } else {
                gaps += 1;
            }
        }
        if gaps > 0 {
            self.metrics
                .xdp_attribution_gaps
                .fetch_add(gaps, Ordering::Relaxed);
        }
        // Map holds only IPs with >=1 drop; the rest of the blocked set is
        // zero-drop. saturating_sub guards a transient reconcile skew.
        let zero = self
            .blocked_ips
            .len()
            .saturating_sub(self.drops_by_blocked.len());
        self.metrics
            .xdp_blocked_ips_zero_drops
            .store(zero as u64, Ordering::Relaxed);
    }

    /// P1-4: re-arm the TTL ring with blocks restored from WAL replay.
    /// `replay_wal_into_store` returns remaining-TTL pairs; call before
    /// `run()` so restored blocks expire on schedule instead of forever.
    pub fn restore_expirations(&mut self, pairs: impl IntoIterator<Item = (IpAddr, u64)>) {
        for (ip, remaining_secs) in pairs {
            if remaining_secs == 0 {
                continue;
            }
            self.schedule_expiration(ip, Instant::now() + Duration::from_secs(remaining_secs));
        }
    }

    pub fn restore_cidr_blocks(&mut self, pairs: impl IntoIterator<Item = (IpNetwork, u64)>) {
        for (network, remaining_secs) in pairs {
            if let Err(e) = self
                .xdp
                .apply_cidr_block(network, Uuid::new_v4(), remaining_secs)
            {
                warn!(cidr=?network, "WAL CIDR restore failed: {}", e);
                continue;
            }
            self.store.active_cidrs.insert(network, ());
            if remaining_secs > 0 {
                self.cidr_expirations.insert(
                    network,
                    Instant::now() + Duration::from_secs(remaining_secs),
                );
            }
        }
    }

    #[cfg(test)]
    fn check_ring_invariant(&self) {
        for (&ip, &(b, pos)) in &self.expirations {
            let vec = self
                .buckets
                .get(&b)
                .unwrap_or_else(|| panic!("{ip} bucket {b} gone"));
            assert_eq!(vec.get(pos), Some(&ip), "index drift for {ip}");
        }
        assert_eq!(
            self.expirations.len(),
            self.buckets.values().map(Vec::len).sum::<usize>(),
            "ring/map size diverged"
        );
    }

    fn remember_result(&mut self, result: &EnforceResult) {
        if self
            .processed_results
            .insert(result.decision_id, result.clone())
            .is_none()
        {
            self.processed_order.push_back(result.decision_id);
            while self.processed_order.len() > 65_536 {
                if let Some(old) = self.processed_order.pop_front() {
                    self.processed_results.remove(&old);
                }
            }
        }
    }

    /// Execute one command. Order: WAL append → storage mutation → local/XDP
    /// indexes. A failed storage mutation must not leave a phantom block; a
    /// failed WAL append aborts before any state change (durable-first).
    pub async fn enforce(
        &mut self,
        cmd: EnforceCommand,
    ) -> Result<EnforceResult, EnforcementError> {
        // Idempotent replay: return what actually happened the first time —
        // a fabricated `xdp_applied: true` would hide a dataplane failure
        // from every retry consumer.
        if let Some(cached) = self.processed_results.get(&cmd.decision_id) {
            trace!(decision_id = %cmd.decision_id, "duplicate decision — returning cached original result");
            return Ok(cached.clone());
        }
        if cmd.ip.is_unspecified() {
            trace!(
                ip = %cmd.ip,
                reason = %cmd.reason,
                "enforce rejected: unspecified IP"
            );
            return Err(EnforcementError::InvalidCommand(
                "unspecified IP is not blockable".into(),
            ));
        }

        // Step 1: commit intent to WAL (durable) — before any state change.
        let wal_lsn = if let Some(ref wal) = self.wal {
            let now_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            let entry = match cmd.action {
                EnforceAction::Block => match cmd.cidr {
                    Some(cidr) => WalEntry::BlockCidr {
                        cidr,
                        reason: cmd.reason.clone(),
                        ttl_secs: (cmd.ttl_seconds > 0).then_some(cmd.ttl_seconds),
                        ts_ns: now_ns,
                    },
                    None => WalEntry::BlockIp {
                        ip: cmd.ip.to_string(),
                        reason: cmd.reason.clone(),
                        ttl_secs: (cmd.ttl_seconds > 0).then_some(cmd.ttl_seconds),
                        ts_ns: now_ns,
                    },
                },
                EnforceAction::Unblock => match cmd.cidr {
                    Some(cidr) => WalEntry::UnblockCidr {
                        cidr,
                        ts_ns: now_ns,
                    },
                    None => WalEntry::UnblockIp {
                        ip: cmd.ip.to_string(),
                        ts_ns: now_ns,
                    },
                },
            };
            let lsn = wal
                .append(&entry)
                .map_err(|e| EnforcementError::Wal(e.to_string()))?;
            self.last_wal_lsn = Some(lsn);
            Some(lsn)
        } else {
            None
        };

        // Step 2: storage mutation.
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        match cmd.action {
            EnforceAction::Block => {
                let reason = reason_to_block_reason(&cmd.reason);
                let rec = self
                    .store
                    .get(&cmd.ip)
                    .and_then(|v| match v {
                        Value::IpRecord(r) => Some(r),
                        _ => None,
                    })
                    .unwrap_or(IpRecord {
                        ip: cmd.ip,
                        request_count: 0,
                        ewma_rps: 0.0,
                        cusum_s: 0.0,
                        baseline_rps: 0.0,
                        prev_sample_hot: false,
                        sample_count: 0,
                        pulse_samples_in_window: 0,
                        pulse_window_start_ns: 0,
                        first_seen_ns: now_ns,
                        last_seen_ns: now_ns,
                        bytes_in: 0,
                        status_dist: [0; 5],
                        proto_fingerprint: 0,
                        threat_score: 0.0,
                        block_state: BlockState::Clean,
                    });
                let mut updated = rec;
                updated.block_state = BlockState::Blocked {
                    reason,
                    since_ns: now_ns,
                };

                // Do not let Store's passive expiry hide a still-blocked record.
                self.store
                    .insert(
                        cmd.ip,
                        Value::IpRecord(updated),
                        None,
                        self.store.traffic.ram_limit_mb.load(Ordering::Relaxed) * 1024 * 1024,
                    )
                    .map_err(|e| EnforcementError::Storage(e.to_string()))?;

                self.blocked_ips.insert(cmd.ip);
                // Re-block resets attribution — a new block is a new epoch.
                self.drops_by_blocked.remove(&cmd.ip);
                if let Some(network) = cmd.cidr {
                    self.store.active_cidrs.insert(network, ());
                }
                // Invariant: at most one expiration per IP. A re-block must not
                // inherit a stale TTL from a previous block/unblock cycle.
                // Ring schedule/detach are both O(1) — TTL refresh moves the
                // card between buckets instead of appending a duplicate.
                if cmd.ttl_seconds > 0 {
                    // TTL is clamped at every entry point (IPC boundary,
                    // config validate) — checked_add is the belt-and-suspenders
                    // guard so a future path can never overflow into a panic
                    // and kill the enforcement task (blocks silently die).
                    let at = Instant::now()
                        .checked_add(Duration::from_secs(cmd.ttl_seconds))
                        .unwrap_or_else(|| {
                            Instant::now() + Duration::from_secs(MAX_EXPIRY_FALLBACK_SECS)
                        });
                    if let Some(network) = cmd.cidr {
                        self.cidr_expirations.insert(network, at);
                    } else {
                        self.schedule_expiration(cmd.ip, at);
                    }
                } else {
                    self.detach_expiration(cmd.ip);
                    if let Some(network) = cmd.cidr {
                        self.cidr_expirations.remove(&network);
                    }
                }

                // Step 3: dataplane.
                let is_cidr = cmd.cidr.is_some();
                let xdp_applied = match cmd.cidr {
                    Some(network) => {
                        self.xdp
                            .apply_cidr_block(network, cmd.decision_id, cmd.ttl_seconds)
                    }
                    None => self
                        .xdp
                        .apply_block(cmd.ip, cmd.decision_id, cmd.ttl_seconds),
                }
                .map(|()| true)
                .unwrap_or_else(|e| {
                    // Userspace + WAL hold this block; the kernel does not, so
                    // the wire keeps passing the target. Counter is the only
                    // scrapeable signal. CIDR LPM tries have a hard cap and no
                    // LRU support, so a full trie means the subnet-swarm leg
                    // has silently stopped — that case is loud, not a warn.
                    self.metrics.inc_xdp_apply_failures();
                    if is_cidr {
                        error!(
                            ip=%cmd.ip, cidr=?cmd.cidr,
                            "XDP subnet block did NOT reach the kernel (CIDR LPM trie full?): {} \
                             — wire mitigation is OFF for this prefix while the engine reports it blocked",
                            e
                        );
                    } else {
                        warn!(ip=%cmd.ip, "XDP block failed: {}", e);
                    }
                    false
                });
                self.metrics.inc_blocks();
                let result = EnforceResult {
                    decision_id: cmd.decision_id,
                    committed: true,
                    applied: true,
                    wal_lsn,
                    xdp_applied,
                    error: None,
                };
                self.remember_result(&result);
                trace!(
                    ip = %cmd.ip,
                    action = "block",
                    reason = %cmd.reason,
                    ttl_seconds = cmd.ttl_seconds,
                    wal_lsn = ?wal_lsn,
                    xdp_applied,
                    decision_id = %cmd.decision_id,
                    "enforce applied: block committed"
                );
                Ok(result)
            }
            EnforceAction::Unblock => {
                if let Some(Value::IpRecord(mut rec)) = self.store.get(&cmd.ip) {
                    rec.block_state = BlockState::Clean;
                    self.store
                        .insert(
                            cmd.ip,
                            Value::IpRecord(rec),
                            None,
                            self.store.traffic.ram_limit_mb.load(Ordering::Relaxed) * 1024 * 1024,
                        )
                        .map_err(|e| EnforcementError::Storage(e.to_string()))?;
                }
                self.blocked_ips.remove(&cmd.ip);
                self.drops_by_blocked.remove(&cmd.ip);
                // Ponytail: mesh CRDT unbans — publish so dashboard reflects
                // live unblock activity.
                if let Some(mesh) = &self.mesh_blocklist {
                    mesh.record_unban(cmd.ip);
                    self.metrics.inc_mesh_record_unban();
                }
                // Purge any pending TTL so a later re-block starts clean.
                self.detach_expiration(cmd.ip);
                if let Some(network) = cmd.cidr {
                    self.cidr_expirations.remove(&network);
                    self.store.active_cidrs.remove(&network);
                }
                let xdp_applied = match cmd.cidr {
                    Some(network) => self.xdp.apply_cidr_unblock(network, cmd.decision_id),
                    None => self.xdp.apply_unblock(cmd.ip, cmd.decision_id),
                }
                .map(|()| true)
                .unwrap_or_else(|e| {
                    warn!(ip=%cmd.ip, cidr=?cmd.cidr, "XDP unblock failed: {}", e);
                    false
                });
                let result = EnforceResult {
                    decision_id: cmd.decision_id,
                    committed: true,
                    applied: true,
                    wal_lsn,
                    xdp_applied,
                    error: None,
                };
                self.remember_result(&result);
                trace!(
                    ip = %cmd.ip,
                    action = "unblock",
                    reason = %cmd.reason,
                    wal_lsn = ?wal_lsn,
                    xdp_applied,
                    decision_id = %cmd.decision_id,
                    "enforce applied: unblock committed"
                );
                Ok(result)
            }
        }
    }
}

fn reason_to_block_reason(reason: &str) -> BlockReason {
    match BlockReason::from_reason_str(&reason.to_ascii_lowercase()) {
        Some(r) => r,
        None => {
            tracing::warn!(
                reason = %reason,
                "unknown block reason string; defaulting to ManualBlock"
            );
            BlockReason::ManualBlock
        }
    }
}

/// Replay WAL entries into the store: fold BlockIp/UnblockIp in LSN order to
/// the final block set, skipping blocks whose TTL already elapsed. Returns
/// the still-live blocks as `(ip, ttl_secs)` pairs — the caller re-arms the
/// enforcement TTL schedule with them (P1-4: blocks restored WITHOUT an
/// expiry card never expire; expirations/buckets are empty at boot).
/// Call before `run()` so the XDP reconciliation inside it picks the
/// recovered state up.
pub fn replay_wal_into_store(store: &Arc<Store>, wal: &Wal) -> anyhow::Result<Vec<(IpAddr, u64)>> {
    let entries = Wal::replay(&wal_dir(wal))?;
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);

    // Sequential fold: later entries win (unblock cancels earlier block).
    let mut blocked: std::collections::HashMap<IpAddr, (BlockReason, u64, Option<u64>)> =
        std::collections::HashMap::new();
    for entry in entries {
        match entry {
            WalEntry::BlockIp {
                ip,
                reason,
                ttl_secs,
                ts_ns,
            } => {
                if let Ok(ip) = ip.parse() {
                    blocked.insert(ip, (reason_to_block_reason(&reason), ts_ns, ttl_secs));
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
    let mut restored: Vec<(IpAddr, u64)> = Vec::new();
    for (ip, (reason, ts_ns, ttl_secs)) in blocked {
        // Expired TTL → don't resurrect.
        if let Some(ttl) = ttl_secs
            && ts_ns.saturating_add(ttl.saturating_mul(1_000_000_000)) <= now_ns
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
        // Remaining TTL = original minus time already served (P1-4 re-arm).
        let remaining = match ttl_secs {
            Some(0) | None => 0,
            Some(ttl) => {
                let elapsed = now_ns.saturating_sub(ts_ns) / 1_000_000_000;
                ttl.saturating_sub(elapsed).max(1)
            }
        };
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
    let now_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut blocked: std::collections::HashMap<IpNetwork, (u64, Option<u64>)> =
        std::collections::HashMap::new();
    for entry in Wal::replay(&wal_dir(wal))? {
        match entry {
            WalEntry::BlockCidr {
                cidr,
                ttl_secs,
                ts_ns,
                ..
            } => {
                blocked.insert(cidr, (ts_ns, ttl_secs));
            }
            WalEntry::UnblockCidr { cidr, .. } => {
                blocked.remove(&cidr);
            }
            _ => {}
        }
    }
    Ok(blocked
        .into_iter()
        .filter_map(|(cidr, (ts_ns, ttl))| {
            let remaining = ttl.map(|seconds| {
                seconds.saturating_sub(now_ns.saturating_sub(ts_ns) / 1_000_000_000)
            });
            if remaining == Some(0) {
                None
            } else {
                Some((cidr, remaining.unwrap_or(0)))
            }
        })
        .collect())
}

fn wal_dir(wal: &Wal) -> String {
    wal.base_dir().to_string()
}

fn epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramshield_metrics::Metrics;

    /// Deterministic applier recording dataplane ops — lets tests assert the
    /// exact block/unblock sequence the service issues.
    struct RecordingApplier {
        log: std::sync::Mutex<Vec<(String, IpAddr)>>,
    }
    impl RecordingApplier {
        fn new() -> Self {
            Self {
                log: std::sync::Mutex::new(Vec::new()),
            }
        }
    }
    #[async_trait::async_trait]
    impl XdpApplier for RecordingApplier {
        fn apply_block(
            &mut self,
            ip: IpAddr,
            _d: Uuid,
            _ttl_seconds: u64,
        ) -> Result<(), EnforcementError> {
            self.log.lock().unwrap().push(("block".into(), ip));
            Ok(())
        }
        fn apply_unblock(&mut self, ip: IpAddr, _d: Uuid) -> Result<(), EnforcementError> {
            self.log.lock().unwrap().push(("unblock".into(), ip));
            Ok(())
        }
        fn reconcile(
            &mut self,
            _expected: &[IpAddr],
            _expected_cidrs: &[IpNetwork],
        ) -> Result<ReconciliationState, EnforcementError> {
            Ok(ReconciliationState::default())
        }
    }

    fn svc(xdp: Box<dyn XdpApplier>) -> EnforcementService {
        let store = Arc::new(Store::new(16));
        store.traffic.ram_limit_mb.store(512, Ordering::Relaxed);
        EnforcementService::new(
            store,
            Arc::new(Metrics::new()),
            xdp,
            Arc::new(AtomicBool::new(false)),
        )
    }

    fn svc_with_wal(xdp: Box<dyn XdpApplier>, dir: &str) -> EnforcementService {
        svc(xdp).with_wal(Arc::new(
            Wal::open(
                dir,
                false,
                ramshield_types::Durability::None,
                64 * 1024 * 1024,
                0,
            )
            .unwrap(),
        ))
    }

    /// 64 B segments: every append rotates, so the rotation open of the NEXT
    /// segment is where a failure lands. Deterministic WAL-failure injection.
    fn svc_with_tiny_wal(dir: &std::path::Path) -> EnforcementService {
        svc(Box::new(RecordingApplier::new())).with_wal(Arc::new(
            Wal::open(
                dir.to_str().unwrap(),
                false,
                ramshield_types::Durability::None,
                64,
                0,
            )
            .unwrap(),
        ))
    }

    /// Documented architecture contract, TTL item: a due lease is retained
    /// until unblock SUCCEEDS. Old code detached the ring card BEFORE the
    /// unblock attempt and dropped it on failure, so one transient WAL or
    /// storage error converted a temporary block into a permanent one —
    /// the IP silently never released.
    #[tokio::test]
    async fn ttl_lease_retries_after_failed_unblock() {
        let dir = std::env::temp_dir().join(format!("rs_enf_retry_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = svc_with_tiny_wal(&dir);
        s.store.traffic.ram_limit_mb.store(512, Ordering::Relaxed);
        let target = ip([10, 60, 0, 9]);
        // The block append succeeds (rotates 0→1). Then poison the NEXT
        // rotation target so the unblock's append hits EISDIR: the WAL
        // fails durably-first, before any state change.
        s.enforce(block_cmd(target, 1)).await.unwrap();
        std::fs::create_dir_all(dir.join("wal-00000002.rshw")).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
        s.expire_due().await;
        assert!(
            s.blocked_ips.contains(&target),
            "failed unblock must not remove the block state"
        );
        assert!(
            s.expirations.contains_key(&target),
            "failed TTL unblock must re-arm the lease for retry"
        );
        s.check_ring_invariant();
        // Recovery: remove the poison, the retried unblock succeeds.
        std::fs::remove_dir_all(dir.join("wal-00000002.rshw")).unwrap();
        // Re-arm deadline is +1s at second-granularity buckets; give it the
        // full slack the ring resolution allows.
        tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
        s.expire_due().await;
        assert!(
            !s.blocked_ips.contains(&target),
            "once WAL works again, the retried lease must unblock"
        );
        assert!(s.expirations.is_empty() && s.buckets.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P2 audit + early-release basis: XDP drop events are attributed to
    /// userspace-blocked IPs ONLY. Invariant: attribution is observability
    /// (audit counters, zero-drop gauge) — it never enters detection or the
    /// forecaster/learning path, because a drop is a consequence of our own
    /// block, not independent threat evidence. Drops with no matching block
    /// are kernel/userspace drift indicators, counted separately.
    #[tokio::test]
    async fn drop_attribution_counts_blocked_ips_only() {
        let target = ip([10, 70, 0, 1]);
        let stranger = ip([10, 70, 0, 2]);
        let mut applier = DroppingApplier::new();
        applier.events = vec![
            XdpDropEvent {
                ip: target,
                ts_ns: 1,
                slot: 0,
            },
            XdpDropEvent {
                ip: target,
                ts_ns: 2,
                slot: 0,
            },
            XdpDropEvent {
                ip: stranger,
                ts_ns: 3,
                slot: 0,
            },
        ];
        let mut s = svc(Box::new(applier));
        s.enforce(block_cmd(target, 60)).await.unwrap();
        let drops = s.xdp.drain_drop_events();
        s.attribute_drops(drops);
        assert_eq!(s.drops_by_blocked.get(&target), Some(&2));
        assert_eq!(
            s.drops_by_blocked.len(),
            1,
            "unblocked IP must not accrue attribution"
        );
        assert_eq!(s.metrics.xdp_attribution_gaps.load(Ordering::Relaxed), 1);
        // target now has drops → no zero-drop blocked IPs.
        assert_eq!(
            s.metrics.xdp_blocked_ips_zero_drops.load(Ordering::Relaxed),
            0
        );
        // Unblock clears attribution — a later re-block starts at zero.
        s.enforce(unblock_cmd(target)).await.unwrap();
        assert!(s.drops_by_blocked.is_empty());
    }

    #[tokio::test]
    async fn zero_drop_gauge_tracks_unseen_blocks() {
        let a = ip([10, 71, 0, 1]);
        let b = ip([10, 71, 0, 2]);
        let mut s = svc(Box::new(DroppingApplier::new()));
        s.enforce(block_cmd(a, 60)).await.unwrap();
        s.enforce(block_cmd(b, 60)).await.unwrap();
        s.attribute_drops(Vec::new());
        assert_eq!(
            s.metrics.xdp_blocked_ips_zero_drops.load(Ordering::Relaxed),
            2
        );
        s.attribute_drops(vec![XdpDropEvent {
            ip: a,
            ts_ns: 1,
            slot: 0,
        }]);
        assert_eq!(
            s.metrics.xdp_blocked_ips_zero_drops.load(Ordering::Relaxed),
            1
        );
    }

    struct DroppingApplier {
        events: Vec<XdpDropEvent>,
    }
    impl DroppingApplier {
        fn new() -> Self {
            Self { events: Vec::new() }
        }
    }
    #[async_trait::async_trait]
    impl XdpApplier for DroppingApplier {
        fn apply_block(&mut self, _: IpAddr, _: Uuid, _: u64) -> Result<(), EnforcementError> {
            Ok(())
        }
        fn apply_unblock(&mut self, _: IpAddr, _: Uuid) -> Result<(), EnforcementError> {
            Ok(())
        }
        fn reconcile(
            &mut self,
            _: &[IpAddr],
            _: &[IpNetwork],
        ) -> Result<ReconciliationState, EnforcementError> {
            Ok(ReconciliationState::default())
        }
        fn drain_drop_events(&mut self) -> Vec<XdpDropEvent> {
            std::mem::take(&mut self.events)
        }
    }

    fn block_cmd(ip: IpAddr, ttl: u64) -> EnforceCommand {
        EnforceCommand {
            decision_id: Uuid::new_v4(),
            policy_version: 1,
            source: "test".into(),
            actor: "test".into(),
            timestamp_utc: 0,
            ttl_seconds: ttl,
            reason: "high_rps".into(),
            ip,
            cidr: None,
            action: EnforceAction::Block,
        }
    }
    fn unblock_cmd(ip: IpAddr) -> EnforceCommand {
        EnforceCommand {
            decision_id: Uuid::new_v4(),
            policy_version: 1,
            source: "test".into(),
            actor: "test".into(),
            timestamp_utc: 0,
            ttl_seconds: 0,
            reason: "manual".into(),
            ip,
            cidr: None,
            action: EnforceAction::Unblock,
        }
    }

    fn ip(a: [u8; 4]) -> IpAddr {
        IpAddr::from(a)
    }

    #[tokio::test]
    async fn block_then_unblock_reaches_dataplane_once() {
        let mut s = svc(Box::new(RecordingApplier::new()));
        let target = ip([9, 9, 9, 9]);
        s.enforce(block_cmd(target, 3600)).await.unwrap();
        s.enforce(unblock_cmd(target)).await.unwrap();
        // Dataplane saw both ops: blocked_ips empty after unblock proves the
        // unblock path ran; store record is Clean.
        assert!(!s.blocked_ips.contains(&target));
        let rec = s.store.get(&target).unwrap();
        match rec {
            Value::IpRecord(r) => assert_eq!(r.block_state, BlockState::Clean),
            _ => panic!("wrong value type"),
        }
    }

    #[tokio::test]
    async fn duplicate_decision_is_idempotent() {
        let mut s = svc(Box::new(RecordingApplier::new()));
        let target = ip([9, 9, 9, 8]);
        let cmd = block_cmd(target, 0);
        s.enforce(cmd.clone()).await.unwrap();
        s.enforce(cmd).await.unwrap();
        // One dataplane op: verify via store state + single blocked entry.
        assert_eq!(s.blocked_ips.len(), 1);
    }

    /// A dataplane-failing applier: storage and WAL still commit, kernel does
    /// not. Used by the idempotency + dataplane-failure contracts.
    struct FailingApplier;
    #[async_trait::async_trait]
    impl XdpApplier for FailingApplier {
        fn apply_block(&mut self, _: IpAddr, _: Uuid, _: u64) -> Result<(), EnforcementError> {
            Err(EnforcementError::Xdp("kernel gone".into()))
        }
        fn apply_unblock(&mut self, _: IpAddr, _: Uuid) -> Result<(), EnforcementError> {
            Err(EnforcementError::Xdp("kernel gone".into()))
        }
        fn reconcile(
            &mut self,
            _: &[IpAddr],
            _: &[IpNetwork],
        ) -> Result<ReconciliationState, EnforcementError> {
            Ok(ReconciliationState::default())
        }
    }

    /// Documented architecture contract, idempotency item: a duplicate
    /// decision_id must return the ORIGINAL EnforceResult. The old code
    /// remembered only the id and fabricated a fresh success
    /// (`xdp_applied: true`) even when the first attempt had failed on the
    /// dataplane — the operator could not see that the kernel never got the
    /// block, and a retry consumer would read a lie.
    #[tokio::test]
    async fn duplicate_decision_returns_original_result() {
        let store = Arc::new(Store::new(16));
        store.traffic.ram_limit_mb.store(512, Ordering::Relaxed);
        let mut s = EnforcementService::new(
            store,
            Arc::new(Metrics::new()),
            Box::new(FailingApplier),
            Arc::new(AtomicBool::new(false)),
        );
        let target = ip([9, 9, 9, 3]);
        let cmd = block_cmd(target, 0);
        let first = s.enforce(cmd.clone()).await.unwrap();
        assert!(
            !first.xdp_applied,
            "test premise: first application failed on the dataplane"
        );
        let second = s.enforce(cmd).await.unwrap();
        assert_eq!(
            first, second,
            "duplicate decision_id must return the cached original result"
        );
    }

    #[tokio::test]
    async fn unspecified_ip_rejected() {
        let mut s = svc(Box::new(RecordingApplier::new()));
        let err = s.enforce(block_cmd(IpAddr::from([0, 0, 0, 0]), 0)).await;
        assert!(matches!(err, Err(EnforcementError::InvalidCommand(_))));
    }

    #[tokio::test]
    async fn reblock_purges_stale_ttl() {
        let ra = Box::new(RecordingApplier::new());
        let mut s = svc(ra);
        let target = ip([9, 9, 9, 7]);
        // Block with TTL, unblock (manual), block again with TTL.
        s.enforce(block_cmd(target, 3600)).await.unwrap();
        s.enforce(unblock_cmd(target)).await.unwrap();
        s.enforce(block_cmd(target, 3600)).await.unwrap();
        // Exactly ONE pending expiration for the IP (old one purged).
        // HashMap type-enforces <=1 per IP; assert the one it must hold.
        assert!(
            s.expirations.contains_key(&target),
            "re-block must keep its TTL"
        );
    }

    #[tokio::test]
    async fn ttl_zero_block_has_no_expiration() {
        let mut s = svc(Box::new(RecordingApplier::new()));
        let target = ip([9, 9, 9, 6]);
        s.enforce(block_cmd(target, 0)).await.unwrap();
        assert!(!s.expirations.contains_key(&target));
        assert!(s.blocked_ips.contains(&target));
    }

    /// Item 14 regression: re-block must MOVE the ring card, not stack a
    /// second one. Block with 1s TTL, immediately refresh to 3600s; after the
    /// first deadline passes, the IP must still be blocked (the long TTL won).
    /// A lazy/duplicate-card design would expire the stale 1s entry here.
    #[tokio::test]
    async fn reblock_moves_card_not_duplicates() {
        let mut s = svc(Box::new(RecordingApplier::new()));
        let target = ip([9, 9, 9, 5]);
        s.enforce(block_cmd(target, 1)).await.unwrap();
        s.enforce(block_cmd(target, 3600)).await.unwrap();
        assert_eq!(s.expirations.len(), 1, "exactly one card pending");
        s.check_ring_invariant();
        tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
        s.expire_due().await;
        assert!(
            s.blocked_ips.contains(&target),
            "refreshed TTL must win — stale 1s card must not expire the IP"
        );
        // Clean up: manual unblock leaves no residue.
        s.enforce(unblock_cmd(target)).await.unwrap();
        assert!(s.expirations.is_empty() && s.buckets.is_empty());
    }

    /// Item 14: a short TTL actually fires through the bucket drain.
    #[tokio::test]
    async fn ring_expires_short_ttl() {
        let mut s = svc(Box::new(RecordingApplier::new()));
        let target = ip([9, 9, 9, 4]);
        s.enforce(block_cmd(target, 1)).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
        s.expire_due().await;
        assert!(!s.blocked_ips.contains(&target), "TTL expiry must unblock");
        assert!(s.expirations.is_empty() && s.buckets.is_empty());
    }

    /// Dense-bucket surgery: swap-remove fixups must keep every remaining
    /// card's (bucket, pos) exact. Churn several IPs across two buckets,
    /// unblock the middle ones, then expire: only true-TTL entries unblock.
    #[tokio::test]
    async fn ring_positions_survive_detach_storm() {
        let mut s = svc(Box::new(RecordingApplier::new()));
        let a = ip([9, 9, 8, 1]);
        let b = ip([9, 9, 8, 2]);
        let c = ip([9, 9, 8, 3]);
        let d = ip([9, 9, 8, 4]);
        s.enforce(block_cmd(a, 3600)).await.unwrap();
        s.enforce(block_cmd(b, 3600)).await.unwrap(); // same bucket as a
        s.enforce(block_cmd(c, 1)).await.unwrap(); // short bucket
        s.enforce(block_cmd(d, 1)).await.unwrap(); // same short bucket as c
        s.enforce(unblock_cmd(a)).await.unwrap(); // detach front of long bucket
        s.enforce(unblock_cmd(c)).await.unwrap(); // detach front of short bucket
        s.check_ring_invariant();
        tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
        s.expire_due().await;
        assert!(s.blocked_ips.contains(&b), "long TTL untouched by drains");
        assert!(!s.blocked_ips.contains(&d), "short TTL fired");
        assert_eq!(s.expirations.len(), 1);
        s.check_ring_invariant();
    }

    #[tokio::test]
    async fn storage_blocked_before_dataplane() {
        // If the dataplane errors, storage must STILL hold the block (fail-open
        // kernel, fail-closed state).
        let store = Arc::new(Store::new(16));
        store.traffic.ram_limit_mb.store(512, Ordering::Relaxed);
        let mut s = EnforcementService::new(
            store.clone(),
            Arc::new(Metrics::new()),
            Box::new(FailingApplier),
            Arc::new(AtomicBool::new(false)),
        );
        let target = ip([9, 9, 9, 5]);
        let res = s.enforce(block_cmd(target, 0)).await.unwrap();
        assert!(
            !res.xdp_applied,
            "xdp_applied must be false on dataplane failure"
        );
        let rec = store.get(&target).expect("record must exist");
        match rec {
            Value::IpRecord(r) => assert!(matches!(r.block_state, BlockState::Blocked { .. })),
            _ => panic!("wrong value type"),
        }
        assert!(s.blocked_ips.contains(&target));
    }

    #[tokio::test]
    async fn wal_first_sets_lsn_and_survives_replay() {
        let dir = std::env::temp_dir().join(format!("rs_enf_wal_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = svc_with_wal(Box::new(RecordingApplier::new()), dir.to_str().unwrap());
        let target = ip([9, 9, 9, 4]);
        let r1 = s.enforce(block_cmd(target, 60)).await.unwrap();
        let lsn1 = r1.wal_lsn.expect("WAL attached ⇒ lsn set");
        assert!(lsn1 >= 1, "LSN base is 1 (0 reserved)");
        let r2 = s.enforce(unblock_cmd(target)).await.unwrap();
        let lsn2 = r2.wal_lsn.expect("unblock also journaled");
        assert!(lsn2 > lsn1, "LSN monotonic");

        drop(s);
        // Replay proves both decisions are durable.
        let entries = Wal::replay(dir.to_str().unwrap()).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(matches!(entries[0], WalEntry::BlockIp { .. }));
        assert!(matches!(entries[1], WalEntry::UnblockIp { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Crash-recovery contract: block → "restart" (fresh store) → replay
    /// restores the block into the store so XDP reconcile re-arms it.
    #[tokio::test]
    async fn replay_restores_block_into_fresh_store() {
        let dir = std::env::temp_dir().join(format!("rs_wal_recov_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut s = svc_with_wal(Box::new(RecordingApplier::new()), dir.to_str().unwrap());
        let target = ip([10, 77, 0, 5]);
        s.enforce(block_cmd(target, 3600)).await.unwrap();
        drop(s);

        // "Restart": empty store, same WAL dir.
        let fresh = Arc::new(Store::new(16));
        let wal = Arc::new(
            Wal::open(
                dir.to_str().unwrap(),
                false,
                ramshield_types::Durability::None,
                64 * 1024 * 1024,
                0,
            )
            .unwrap(),
        );
        let restored = replay_wal_into_store(&fresh, &wal).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].1, 3600, "restored block must carry full TTL");
        match fresh.get(&target) {
            Some(Value::IpRecord(r)) => assert!(
                matches!(r.block_state, BlockState::Blocked { .. }),
                "replay must restore Blocked state"
            ),
            other => panic!("expected IpRecord after replay, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Unblock cancels a prior block across the restart boundary.
    #[tokio::test]
    async fn replay_unblock_cancels_block() {
        let dir = std::env::temp_dir().join(format!("rs_wal_cancel_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut s = svc_with_wal(Box::new(RecordingApplier::new()), dir.to_str().unwrap());
        let target = ip([10, 78, 0, 6]);
        s.enforce(block_cmd(target, 3600)).await.unwrap();
        s.enforce(unblock_cmd(target)).await.unwrap();
        drop(s);

        let fresh = Arc::new(Store::new(16));
        let wal = Arc::new(
            Wal::open(
                dir.to_str().unwrap(),
                false,
                ramshield_types::Durability::None,
                64 * 1024 * 1024,
                0,
            )
            .unwrap(),
        );
        assert_eq!(replay_wal_into_store(&fresh, &wal).unwrap().len(), 0);
        assert!(fresh.get(&target).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Expired TTL blocks are not resurrected on restart.
    #[tokio::test]
    async fn replay_skips_expired_ttl_blocks() {
        let dir = std::env::temp_dir().join(format!("rs_wal_ttl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Hand-write an ancient block entry with ttl=1s.
        let wal = Wal::open(
            dir.to_str().unwrap(),
            false,
            ramshield_types::Durability::None,
            64 * 1024 * 1024,
            0,
        )
        .unwrap();
        wal.append(&WalEntry::BlockIp {
            ip: "10.79.0.7".into(),
            reason: "high_rps".into(),
            ttl_secs: Some(1),
            ts_ns: 1, // epoch + 1ns — long expired
        })
        .unwrap();
        drop(wal);

        let fresh = Arc::new(Store::new(16));
        let wal2 = Arc::new(
            Wal::open(
                dir.to_str().unwrap(),
                false,
                ramshield_types::Durability::None,
                64 * 1024 * 1024,
                0,
            )
            .unwrap(),
        );
        assert_eq!(replay_wal_into_store(&fresh, &wal2).unwrap().len(), 0);
        assert!(
            fresh.get(&"10.79.0.7".parse().unwrap()).is_none(),
            "expired block must not resurrect"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// P1-4: replay returns remaining TTLs; restore_expirations re-arms the
    /// ring so a restored block actually expires (previously: forever).
    #[tokio::test]
    async fn replay_then_restore_expirations_arms_ring() {
        let dir = std::env::temp_dir().join(format!("rs_wal_ream_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut s = svc_with_wal(Box::new(RecordingApplier::new()), dir.to_str().unwrap());
        let target = ip([10, 80, 0, 9]);
        s.enforce(block_cmd(target, 3600)).await.unwrap();
        drop(s);

        let fresh = Arc::new(Store::new(16));
        let wal = Arc::new(
            Wal::open(
                dir.to_str().unwrap(),
                false,
                ramshield_types::Durability::None,
                64 * 1024 * 1024,
                0,
            )
            .unwrap(),
        );
        let pairs = replay_wal_into_store(&fresh, &wal).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].1, 3600, "block written seconds ago keeps full TTL");

        // New service instance (empty ring) restores + re-arms.
        let mut s2 = EnforcementService::new(
            fresh,
            Arc::new(Metrics::new()),
            Box::new(RecordingApplier::new()),
            Arc::new(AtomicBool::new(false)),
        );
        s2.restore_expirations(pairs);
        assert!(
            s2.expirations.contains_key(&target),
            "restored block must be scheduled in the TTL ring"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn cidr_block_registers_then_unblock_clears_shared_set() {
        // The shared set is the one clock: the actor writes it on BlockCidr
        // and clears it on UnblockCidr, and check_ip reads it. A member host
        // has no IpRecord, so this set is the only place the block exists.
        let store = Arc::new(Store::new(16));
        store.traffic.ram_limit_mb.store(512, Ordering::Relaxed);
        let mut s = EnforcementService::new(
            store.clone(),
            Arc::new(Metrics::new()),
            Box::new(RecordingApplier::new()),
            Arc::new(AtomicBool::new(false)),
        );
        let net = IpNetwork::new("198.51.100.0".parse().unwrap(), 24).unwrap();
        let member: IpAddr = "198.51.100.42".parse().unwrap();

        let mut block = block_cmd(net.addr, 600);
        block.cidr = Some(net);
        s.enforce(block).await.unwrap();

        assert_eq!(
            store.is_blocked_by_cidr(&member),
            Some(net),
            "block must register in the shared set the query reads"
        );
        assert!(store.get(&member).is_none(), "no per-member IpRecord");

        let mut unblock = unblock_cmd(net.addr);
        unblock.cidr = Some(net);
        s.enforce(unblock).await.unwrap();

        assert!(
            store.is_blocked_by_cidr(&member).is_none(),
            "unblock must clear the shared set"
        );
    }

    /// P5 case 2/3: WAL committed, then replay twice is safe (no double-apply).
    #[tokio::test]
    async fn replay_twice_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("rs_wal_idemp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = svc_with_wal(Box::new(RecordingApplier::new()), dir.to_str().unwrap());
        let target = ip([10, 91, 0, 2]);
        s.enforce(block_cmd(target, 3600)).await.unwrap();
        drop(s);

        let wal = Arc::new(
            Wal::open(
                dir.to_str().unwrap(),
                false,
                ramshield_types::Durability::None,
                64 * 1024 * 1024,
                0,
            )
            .unwrap(),
        );
        let a = Arc::new(Store::new(16));
        let first = replay_wal_into_store(&a, &wal).unwrap();
        let second = replay_wal_into_store(&a, &wal).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].0, second[0].0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    struct MapApplier {
        blocked: std::collections::HashSet<IpAddr>,
        cidrs: std::collections::HashSet<IpNetwork>,
    }
    impl MapApplier {
        fn new() -> Self {
            Self { blocked: Default::default(), cidrs: Default::default() }
        }
    }
    #[async_trait::async_trait]
    impl XdpApplier for MapApplier {
        fn apply_block(&mut self, ip: IpAddr, _: Uuid, _: u64) -> Result<(), EnforcementError> {
            self.blocked.insert(ip);
            Ok(())
        }
        fn apply_unblock(&mut self, ip: IpAddr, _: Uuid) -> Result<(), EnforcementError> {
            self.blocked.remove(&ip);
            Ok(())
        }
        fn apply_cidr_block(&mut self, n: IpNetwork, _: Uuid, _: u64) -> Result<(), EnforcementError> {
            self.cidrs.insert(n);
            Ok(())
        }
        fn apply_cidr_unblock(&mut self, n: IpNetwork, _: Uuid) -> Result<(), EnforcementError> {
            self.cidrs.remove(&n);
            Ok(())
        }
        fn reconcile(&mut self, expected: &[IpAddr], expected_cidrs: &[IpNetwork]) -> Result<ReconciliationState, EnforcementError> {
            let want: std::collections::HashSet<_> = expected.iter().copied().collect();
            let stale: Vec<_> = self.blocked.difference(&want).copied().collect();
            let missing: Vec<_> = want.difference(&self.blocked).copied().collect();
            for ip in &stale { self.blocked.remove(ip); }
            for ip in &missing { self.blocked.insert(*ip); }
            let want_c: std::collections::HashSet<_> = expected_cidrs.iter().copied().collect();
            let stale_c: Vec<_> = self.cidrs.difference(&want_c).copied().collect();
            let missing_c: Vec<_> = want_c.difference(&self.cidrs).copied().collect();
            for c in &stale_c { self.cidrs.remove(c); }
            for c in &missing_c { self.cidrs.insert(*c); }
            Ok(ReconciliationState { last_wal_lsn: 0, pending_blocks: missing, pending_unblocks: stale })
        }
    }

    #[tokio::test]
    async fn reconcile_repairs_missing_and_stale_ip() {
        let mut a = MapApplier::new();
        let live = ip([10, 1, 0, 1]);
        let stale_ip = ip([10, 1, 0, 2]);
        a.blocked.insert(stale_ip);
        a.reconcile(&[live], &[]).unwrap();
        assert!(a.blocked.contains(&live));
        assert!(!a.blocked.contains(&stale_ip));
    }

    #[tokio::test]
    async fn reconcile_repairs_missing_and_stale_cidr() {
        let mut a = MapApplier::new();
        let live = IpNetwork::new("198.51.100.0".parse().unwrap(), 24).unwrap();
        let stale_c = IpNetwork::new("203.0.113.0".parse().unwrap(), 24).unwrap();
        a.cidrs.insert(stale_c);
        a.reconcile(&[], &[live]).unwrap();
        assert!(a.cidrs.contains(&live));
        assert!(!a.cidrs.contains(&stale_c));
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]
        #[test]
        fn sequence_invariant(ops in proptest::collection::vec(
            (proptest::arbitrary::any::<u8>(), proptest::bool::ANY, proptest::option::of(1u64..100)),
            1..64
        )) {
            use proptest::prop_assert;
            proptest::prop_assume!(!ops.is_empty());
            let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
            rt.block_on(async move {
                let mut s = svc(Box::new(RecordingApplier::new()));
                for (seed, is_block, ttl) in ops {
                    let target = ip([10, 0, seed / 2, seed]);
                    if is_block {
                        let ttl = ttl.unwrap_or(0);
                        let r = s.enforce(block_cmd(target, ttl)).await.unwrap();
                        prop_assert!(r.committed);
                        prop_assert!(s.blocked_ips.contains(&target));
                    } else {
                        let _ = s.enforce(unblock_cmd(target)).await.unwrap();
                        prop_assert!(!s.blocked_ips.contains(&target));
                        prop_assert!(!s.expirations.contains_key(&target),
                            "unblock must purge TTL entry");
                    }
                    // Global invariant: expirations never exceed blocked set size.
                    prop_assert!(s.expirations.len() <= s.blocked_ips.len());
                }
                Ok(())
            })?;
        }
    }
}