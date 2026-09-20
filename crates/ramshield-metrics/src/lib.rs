use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use sysinfo::System;

const HISTORY: usize = 80;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn with_system<F, R>(f: F) -> R
where
    F: FnOnce(&mut System) -> R,
{
    static SYS: Mutex<Option<System>> = Mutex::new(None);
    // ponytail: poison-recovery — sysinfo cache is advisory; a panicked
    // holder must not take down every dashboard poll. Upgrade: parking_lot.
    let mut guard = SYS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.is_none() {
        *guard = Some(System::new_all());
    }
    match guard.as_mut() {
        Some(sys) => f(sys),
        None => unreachable!("SYS initialized above"),
    }
}

/// (cpu_usage, total_ram_mb, own_process_rss_mb). Cached 1s — see get_system_usage.
pub fn get_system_usage() -> (f32, usize, usize) {
    // ponytail: 1s TTL cache — dashboard polls snapshot+modules per cycle and
    // both need the same numbers; upgrade to crossbeam channel ticker if
    // sub-second freshness ever matters.
    static CACHE: Mutex<Option<(std::time::Instant, f32, usize, usize)>> = Mutex::new(None);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, cpu, mem, rss)) =
        (*cache).filter(|(at, ..)| at.elapsed() < std::time::Duration::from_secs(1))
    {
        return (cpu, mem, rss);
    }
    let fresh = with_system(|sys| {
        // CPU% needs two samples spaced ~200ms+; refresh_specifics avoids the
        // full process-table walk of refresh_all() on every dashboard poll.
        sys.refresh_specifics(
            sysinfo::RefreshKind::nothing()
                .with_cpu(sysinfo::CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(sysinfo::MemoryRefreshKind::everything()),
        );
        let cpu_usage = sys.global_cpu_usage();
        // sysinfo 0.30+: total_memory() returns bytes (was KB before).
        let total_memory_mb = (sys.total_memory() / (1024 * 1024)) as usize;
        let rss_mb = sys
            .process(
                sysinfo::get_current_pid()
                    .ok()
                    .unwrap_or(sysinfo::Pid::from(0)),
            )
            .map(|p| p.memory() / (1024 * 1024))
            .unwrap_or(0) as usize;
        (cpu_usage, total_memory_mb, rss_mb)
    });
    *cache = Some((std::time::Instant::now(), fresh.0, fresh.1, fresh.2));
    fresh
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchRecord {
    pub ts_ms: u64,
    pub events: u32,
    pub unique_ips: u32,
    /// Unique IPs promoted to full tracking
    pub promoted: u32,
    /// Unique IPs skipped (below promotion threshold)
    pub cold_skipped: u32,
    /// Connection events in promoted IPs
    pub promoted_events: u32,
    /// Connection events in cold-skipped IPs
    pub cold_skipped_events: u32,
    pub blocks: u32,
    pub hot_subnets: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockRecord {
    pub ts_ms: u64,
    pub ip: String,
    pub reason: String,
    pub module: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleStats {
    pub label: String,
    pub events: u64,
    pub errors: u64,
    pub rate_per_sec: f64,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub ts_ms: u64,
    pub uptime_secs: u64,
    pub ips_tracked: usize,
    pub blocked_total: u64,
    pub ram_bytes: usize,
    pub ram_limit_mb: usize,
    pub ram_pct: f64,
    pub cpu_usage: f32,
    pub memory_usage_mb: usize,
    pub total_ram_mb: usize,
    pub ipc_requests: u64,
    pub events_ingested: u64,
    pub events_rejected: u64,
    pub frames_rejected_total: u64,
    pub channel_depth: usize,
    pub events_shed: u64,
    pub batches_total: u64,
    pub promotions: u64,
    pub cold_skipped: u64,
    pub blocks_applied: u64,
    pub pipeline: PipelineFlow,
    pub is_healthy: bool,
    pub health_reason: String,
    /// True if the kernel XDP dataplane is loaded and attached. False when
    /// the daemon is running in degraded mode (in-band enforcement only).
    pub xdp_active: bool,
    /// Last committed WAL LSN (0 = no WAL configured).
    pub wal_lsn: u64,
    /// Pending TTL expirations in the enforcement ring.
    pub pending_expirations: u64,
}

impl Default for DashboardSnapshot {
    fn default() -> Self {
        Self {
            ts_ms: 0,
            uptime_secs: 0,
            ips_tracked: 0,
            blocked_total: 0,
            ram_bytes: 0,
            ram_limit_mb: 0,
            ram_pct: 0.0,
            cpu_usage: 0.0,
            memory_usage_mb: 0,
            total_ram_mb: 0,
            ipc_requests: 0,
            events_ingested: 0,
            events_rejected: 0,
            frames_rejected_total: 0,
            channel_depth: 0,
            events_shed: 0,
            batches_total: 0,
            promotions: 0,
            cold_skipped: 0,
            blocks_applied: 0,
            pipeline: PipelineFlow {
                ingest: 0,
                queued: 0,
                batched: 0,
                promoted: 0,
                merged: 0,
                blocked: 0,
            },
            is_healthy: true,
            health_reason: "initializing".to_string(),
            xdp_active: false,
            wal_lsn: 0,
            pending_expirations: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubnetRow {
    pub prefix: String,
    pub events: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineFlow {
    pub ingest: u64,
    pub queued: u64,
    pub batched: u64,
    pub promoted: u64,
    pub merged: u64,
    pub blocked: u64,
}

pub struct Metrics {
    pub requests_total: Arc<AtomicU64>,
    pub blocks_total: Arc<AtomicU64>,
    pub events_ingested: Arc<AtomicU64>,
    pub events_rejected: Arc<AtomicU64>,
    pub frames_rejected: Arc<AtomicU64>,
    /// Low-signal events shed at the IPC high-water mark to preserve space
    /// for attack telemetry (status>=400, anomalous fp, >64 KiB).
    pub events_shed: Arc<AtomicU64>,
    /// Instantaneous bounded ingest-queue occupancy (gauge; written by the
    /// engine snapshot path — see Engine::dashboard_snapshot / get_module_stats).
    pub ingest_channel_depth: Arc<AtomicU64>,
    /// Userspace mirror of active CIDR LPM blocks (gauge; written by the
    /// enforcement tick). Kernel BLOCKCIDR maps are authoritative; mirror
    /// exists for cheap telemetry without per-tick map iteration.
    pub active_cidr_blocks: Arc<AtomicU64>,
    pub batches_total: Arc<AtomicU64>,
    pub promotions_total: Arc<AtomicU64>,
    pub cold_skipped_total: Arc<AtomicU64>,
    pub blocks_detection: Arc<AtomicU64>,
    pub blocks_subnet: Arc<AtomicU64>,
    pub blocks_forecast: Arc<AtomicU64>,
    /// Block commands dropped because the detection→enforcement channel was
    /// full. A dropped security command means the attacker keeps flooding
    /// while the engine believes the IP is blocked; this counter is the only
    /// operator-visible signal that happened.
    pub enforcement_dropped: Arc<AtomicU64>,
    /// Track when detection updates fail due to capacity exceeded
    pub capacity_exceeded_count: Arc<AtomicU64>,
    /// Track how many IPs fail due to capacity exceeded
    pub capacity_exceeded_ips: Arc<AtomicU64>,
    pub forecast_ticks: Arc<AtomicU64>,
    pub entropy_ticks: Arc<AtomicU64>,
    pub hw_rps_bits: Arc<AtomicU64>,
    pub hw_z_bits: Arc<AtomicU64>,
    pub hw_forecast_bits: Arc<AtomicU64>,
    pub entropy_bits: Arc<AtomicU64>,
    /// P2: CGNAT graduated mitigation counters
    pub cgnat_classify_ticks: Arc<AtomicU64>,
    pub cgnat_tier_allow: Arc<AtomicU64>,
    pub cgnat_tier_challenge: Arc<AtomicU64>,
    pub cgnat_tier_powdrop: Arc<AtomicU64>,
    pub cgnat_tier_block: Arc<AtomicU64>,
    /// P2: SHM rule table publish counters
    pub shm_publish_count: Arc<AtomicU64>,
    pub shm_lookup_count: Arc<AtomicU64>,
    pub shm_cache_hits: Arc<AtomicU64>,
    /// P2: Analytics streaming counters
    pub hll_insert_count: Arc<AtomicU64>,
    pub cms_increment_count: Arc<AtomicU64>,
    pub cms_decay_ticks: Arc<AtomicU64>,
    /// XDP kernel dataplane counters (from AyaXdpApplier::counters())
    /// COUNTERS PerCpuArray: [v4_drop, v6_drop, wire_pass, parse_fail]
    pub xdp_v4_drops: Arc<AtomicU64>,
    pub xdp_v6_drops: Arc<AtomicU64>,
    pub xdp_wire_pass: Arc<AtomicU64>,
    pub xdp_parse_fails: Arc<AtomicU64>,
    /// XDP apply attempts that failed. The block is held in userspace (and
    /// WAL) but NEVER reached the kernel, so the wire keeps passing the
    /// attacker while the engine believes the target is blocked. Only the
    /// CIDR LPM tries can exhaust (102_400 hard cap, no LRU support); the
    /// per-IP maps are LRU and self-evict, so ENOSPC there is impossible.
    /// Without this counter a full trie is a silent loss of the subnet-
    /// swarm mitigation leg — a warn! log and nothing scrapeable.
    pub xdp_apply_failures: Arc<AtomicU64>,
    /// Enforcement: last committed WAL LSN (atomic read for dashboard).
    pub wal_lsn: Arc<AtomicU64>,
    /// Enforcement: pending TTL expirations count.
    pub pending_expirations: Arc<AtomicU64>,
    /// P3: Mesh CRDT counters
    pub mesh_record_ban_count: Arc<AtomicU64>,
    pub mesh_record_unban_count: Arc<AtomicU64>,
    pub mesh_purge_ticks: Arc<AtomicU64>,
    pub mesh_hlc_ticks: Arc<AtomicU64>,
    pub last_batch_events: Arc<AtomicU64>,
    pub last_batch_promoted: Arc<AtomicU64>,
    pub last_batch_blocks: Arc<AtomicU64>,
    /// Bloom advisory-cache observability (Patch A). The bloom is a revisit
    /// cache for *promoted* IPs, not a block list, so its fill must be
    /// measurable: `bloom_inserts_epoch` counts distinct promotes since the
    /// last 8s clear; `bloom_fp_ppm` is derived in render_prometheus from
    /// n and m (k=2). FP only ever OPENS the promote gate (over-promote),
    /// never rejects — so an over-full bloom is a tuning signal, not an
    /// outage.
    pub bloom_bits: Arc<AtomicU64>,
    pub bloom_inserts_epoch: Arc<AtomicU64>,
    pub bloom_clears_total: Arc<AtomicU64>,
    pub last_batch: Arc<Mutex<Option<Arc<BatchRecord>>>>,
    pub batch_history: Arc<Mutex<VecDeque<Arc<BatchRecord>>>>,
    pub block_log: Arc<Mutex<VecDeque<BlockRecord>>>,
    pub block_log_cap: usize,
    /// Write-path sequence for the JSON caches (RAM-for-CPU item 16): bumped
    /// under the same mutex the ring mutation holds, read by get_*_json.
    /// Plain atomics — Metrics itself always lives behind an Arc.
    block_seq: AtomicU64,
    batch_seq: AtomicU64,
    // Per-instance caches, not function statics: several Metrics objects
    // exist in tests/embedded use; a shared static leaks one instance's text.
    metrics_cache: Mutex<Option<(std::time::Instant, Arc<str>)>>,
    blocks_json_cache: Mutex<Option<(u64, Arc<str>)>>,
    batches_json_cache: Mutex<Option<(u64, Arc<str>)>>,
    started_ms: u64,
}

impl Metrics {
    pub fn new() -> Self {
        Self::with_block_log(1_000)
    }

    /// `block_log_size`: ring size served by `/api/history/blocks`.
    /// Was a hardcoded 40 — useless during floods. Config-driven now
    /// (`[dashboard] block_log_size`, default 1000).
    pub fn with_block_log(block_log_size: usize) -> Self {
        Self {
            requests_total: Arc::new(AtomicU64::new(0)),
            blocks_total: Arc::new(AtomicU64::new(0)),
            events_ingested: Arc::new(AtomicU64::new(0)),
            events_rejected: Arc::new(AtomicU64::new(0)),
            frames_rejected: Arc::new(AtomicU64::new(0)),
            events_shed: Arc::new(AtomicU64::new(0)),
            ingest_channel_depth: Arc::new(AtomicU64::new(0)),
            active_cidr_blocks: Arc::new(AtomicU64::new(0)),
            batches_total: Arc::new(AtomicU64::new(0)),
            promotions_total: Arc::new(AtomicU64::new(0)),
            cold_skipped_total: Arc::new(AtomicU64::new(0)),
            blocks_detection: Arc::new(AtomicU64::new(0)),
            blocks_subnet: Arc::new(AtomicU64::new(0)),
            blocks_forecast: Arc::new(AtomicU64::new(0)),
            enforcement_dropped: Arc::new(AtomicU64::new(0)),
            capacity_exceeded_count: Arc::new(AtomicU64::new(0)),
            capacity_exceeded_ips: Arc::new(AtomicU64::new(0)),
            forecast_ticks: Arc::new(AtomicU64::new(0)),
            entropy_ticks: Arc::new(AtomicU64::new(0)),
            hw_rps_bits: Arc::new(AtomicU64::new(0)),
            hw_z_bits: Arc::new(AtomicU64::new(0)),
            hw_forecast_bits: Arc::new(AtomicU64::new(0)),
            entropy_bits: Arc::new(AtomicU64::new(0)),
            cgnat_classify_ticks: Arc::new(AtomicU64::new(0)),
            cgnat_tier_allow: Arc::new(AtomicU64::new(0)),
            cgnat_tier_challenge: Arc::new(AtomicU64::new(0)),
            cgnat_tier_powdrop: Arc::new(AtomicU64::new(0)),
            cgnat_tier_block: Arc::new(AtomicU64::new(0)),
            shm_publish_count: Arc::new(AtomicU64::new(0)),
            shm_lookup_count: Arc::new(AtomicU64::new(0)),
            shm_cache_hits: Arc::new(AtomicU64::new(0)),
            hll_insert_count: Arc::new(AtomicU64::new(0)),
            cms_increment_count: Arc::new(AtomicU64::new(0)),
            cms_decay_ticks: Arc::new(AtomicU64::new(0)),
            xdp_v4_drops: Arc::new(AtomicU64::new(0)),
            xdp_v6_drops: Arc::new(AtomicU64::new(0)),
            xdp_wire_pass: Arc::new(AtomicU64::new(0)),
            xdp_parse_fails: Arc::new(AtomicU64::new(0)),
            xdp_apply_failures: Arc::new(AtomicU64::new(0)),
            wal_lsn: Arc::new(AtomicU64::new(0)),
            pending_expirations: Arc::new(AtomicU64::new(0)),
            mesh_record_ban_count: Arc::new(AtomicU64::new(0)),
            mesh_record_unban_count: Arc::new(AtomicU64::new(0)),
            mesh_purge_ticks: Arc::new(AtomicU64::new(0)),
            mesh_hlc_ticks: Arc::new(AtomicU64::new(0)),
            last_batch_events: Arc::new(AtomicU64::new(0)),
            last_batch_promoted: Arc::new(AtomicU64::new(0)),
            last_batch_blocks: Arc::new(AtomicU64::new(0)),
            bloom_bits: Arc::new(AtomicU64::new(0)),
            bloom_inserts_epoch: Arc::new(AtomicU64::new(0)),
            bloom_clears_total: Arc::new(AtomicU64::new(0)),
            last_batch: Arc::new(Mutex::new(None)),
            batch_history: Arc::new(Mutex::new(VecDeque::with_capacity(HISTORY))),
            block_log: Arc::new(Mutex::new(VecDeque::with_capacity(block_log_size.max(1)))),
            block_log_cap: block_log_size.max(1),
            block_seq: AtomicU64::new(0),
            batch_seq: AtomicU64::new(0),
            metrics_cache: Mutex::new(None),
            blocks_json_cache: Mutex::new(None),
            batches_json_cache: Mutex::new(None),
            started_ms: now_ms(),
        }
    }

    pub fn inc_requests(&self) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_blocks(&self) {
        self.blocks_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_ingested(&self, n: u64) {
        self.events_ingested.fetch_add(n, Ordering::Relaxed);
    }
    pub fn inc_rejected(&self, n: u64) {
        self.events_rejected.fetch_add(n, Ordering::Relaxed);
    }
    pub fn inc_frames_rejected(&self) {
        self.frames_rejected.fetch_add(1, Ordering::Relaxed);
    }
    /// A block command was dropped because the detection→enforcement channel
    /// was full. Distinct from `capacity_exceeded_*` (store RAM refusal):
    /// here the IP WAS blocked in memory but the kernel never learned.
    pub fn inc_enforcement_dropped(&self) {
        self.enforcement_dropped.fetch_add(1, Ordering::Relaxed);
    }
    /// A block/unblock decision failed to reach the kernel (map insert error:
    /// CIDR LPM trie full, or a transient XDP load/attach failure). Userspace
    /// and the WAL still hold it; the wire does not. Mirror of
    /// inc_enforcement_dropped for the dataplane leg.
    pub fn inc_xdp_apply_failures(&self) {
        self.xdp_apply_failures.fetch_add(1, Ordering::Relaxed);
    }
    /// Low-signal events shed at the IPC high-water mark to preserve space
    /// for attack telemetry (status>=400, anomalous fingerprint, >64 KiB).
    pub fn inc_shed(&self, n: u64) {
        self.events_shed.fetch_add(n, Ordering::Relaxed);
    }

    /// Patch A: total bloom capacity in bits (gauge). Written once at
    /// detection construction; `bloom_fp_ppm` is derived from it at render.
    pub fn set_bloom_bits(&self, bits: u64) {
        self.bloom_bits.store(bits, Ordering::Relaxed);
    }

    /// Patch A: add this flush's distinct promotes to the current epoch.
    /// `n` is the promoted count, NOT the block count — the bloom tracks
    /// promoted IPs so a promote-without-block host is still revisitable.
    pub fn record_bloom_inserts(&self, n: u64) {
        self.bloom_inserts_epoch.fetch_add(n, Ordering::Relaxed);
    }

    /// Patch A: 8s bloom epoch ended. Zero the insert counter so
    /// `bloom_fp_ppm` returns to 0, and bump the clears counter.
    pub fn bloom_epoch_clear(&self) {
        self.bloom_inserts_epoch.store(0, Ordering::Relaxed);
        self.bloom_clears_total.fetch_add(1, Ordering::Relaxed);
    }
    /// Gauge: bounded ingest-queue occupancy. Written by the engine snapshot
    /// path (dashboard refresh + SSE), same source value on both writers.
    pub fn set_channel_depth(&self, depth: usize) {
        self.ingest_channel_depth
            .store(depth as u64, Ordering::Relaxed);
    }
    /// Gauge: userspace mirror of active CIDR LPM blocks. Written by the
    /// enforcement 250 ms tick from EnforcementService::active_cidrs.
    pub fn set_active_cidr_blocks(&self, n: usize) {
        self.active_cidr_blocks.store(n as u64, Ordering::Relaxed);
    }
    pub fn set_xdp_counters(&self, v4_drop: u64, v6_drop: u64, pass: u64, parse_fail: u64) {
        self.xdp_v4_drops.store(v4_drop, Ordering::Relaxed);
        self.xdp_v6_drops.store(v6_drop, Ordering::Relaxed);
        self.xdp_wire_pass.store(pass, Ordering::Relaxed);
        self.xdp_parse_fails.store(parse_fail, Ordering::Relaxed);
    }
    pub fn set_wal_lsn(&self, lsn: u64) {
        self.wal_lsn.store(lsn, Ordering::Relaxed);
    }
    pub fn set_pending_expirations(&self, n: u64) {
        self.pending_expirations.store(n, Ordering::Relaxed);
    }
    // ponytail: P2/P3 module counters — writer methods so the dashboard reads
    // live values instead of dead zeros. Each caller owns its Arc<Metrics>
    // and calls the matching inc_* on the hot path.

    /// CGNAT graduated-mitigation verdicts.
    pub fn inc_cgnat_classify(&self) {
        self.cgnat_classify_ticks.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_cgnat_allow(&self) {
        self.cgnat_tier_allow.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_cgnat_challenge(&self) {
        self.cgnat_tier_challenge.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_cgnat_powdrop(&self) {
        self.cgnat_tier_powdrop.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_cgnat_block(&self) {
        self.cgnat_tier_block.fetch_add(1, Ordering::Relaxed);
    }

    /// Analytics streaming counters.
    pub fn inc_hll_insert(&self) {
        self.hll_insert_count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_hll_inserts(&self, n: u64) {
        self.hll_insert_count.fetch_add(n, Ordering::Relaxed);
    }
    pub fn inc_cms_increment(&self) {
        self.cms_increment_count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_cms_increments(&self, n: u64) {
        self.cms_increment_count.fetch_add(n, Ordering::Relaxed);
    }
    pub fn inc_shm_publish(&self) {
        self.shm_publish_count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_shm_lookup(&self) {
        self.shm_lookup_count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_shm_cache_hit(&self) {
        self.shm_cache_hits.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_cms_decay(&self) {
        self.cms_decay_ticks.fetch_add(1, Ordering::Relaxed);
    }

    /// Mesh CRDT counters.
    pub fn inc_mesh_record_ban(&self) {
        self.mesh_record_ban_count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_mesh_record_unban(&self) {
        self.mesh_record_unban_count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_mesh_purge(&self) {
        self.mesh_purge_ticks.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_mesh_hlc(&self) {
        self.mesh_hlc_ticks.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_batch(&self, rec: BatchRecord) {
        self.batches_total.fetch_add(1, Ordering::Relaxed);
        self.last_batch_events
            .store(rec.events as u64, Ordering::Relaxed);
        self.last_batch_promoted
            .store(rec.promoted as u64, Ordering::Relaxed);
        self.last_batch_blocks
            .store(rec.blocks as u64, Ordering::Relaxed);
        self.promotions_total
            .fetch_add(rec.promoted as u64, Ordering::Relaxed);
        self.cold_skipped_total
            .fetch_add(rec.cold_skipped as u64, Ordering::Relaxed);
        self.blocks_detection
            .fetch_add(rec.blocks as u64, Ordering::Relaxed);
        // ponytail: Arc avoids the full BatchRecord clone (~64 B) on every batch.
        let shared = Arc::new(rec);
        if let Ok(mut h) = self.batch_history.lock() {
            if h.len() >= HISTORY {
                h.pop_front();
            }
            h.push_back(Arc::clone(&shared));
            self.batch_seq.fetch_add(1, Ordering::Release);
        }
        if let Ok(mut lb) = self.last_batch.lock() {
            *lb = Some(shared);
        }
    }

    pub fn record_block(&self, ip: &str, reason: &str, module: &str) {
        if let Ok(mut log) = self.block_log.lock() {
            while log.len() >= self.block_log_cap {
                log.pop_front();
            }
            log.push_back(BlockRecord {
                ts_ms: now_ms(),
                ip: ip.to_string(),
                reason: reason.to_string(),
                module: module.to_string(),
            });
            self.block_seq.fetch_add(1, Ordering::Release);
        }
    }

    /// Record a block event with `IpAddr`, avoiding caller-side `to_string()`.
    #[inline]
    pub fn record_block_ip(&self, ip: &IpAddr, reason: &str, module: &str) {
        if let Ok(mut log) = self.block_log.lock() {
            while log.len() >= self.block_log_cap {
                log.pop_front();
            }
            // ponytail: avoid double-format of IpAddr — caller used to call
            // `record_block(&ip.to_string(), ...)` which allocated a throwaway
            // String just to feed it to `to_string()` again inside.
            log.push_back(BlockRecord {
                ts_ms: now_ms(),
                ip: ip.to_string(),
                reason: reason.to_string(),
                module: module.to_string(),
            });
            self.block_seq.fetch_add(1, Ordering::Release);
        }
    }

    pub fn set_forecast_hw(&self, rps: f64, z: f64, forecast: f64) {
        self.hw_rps_bits.store(rps.to_bits(), Ordering::Relaxed);
        self.hw_z_bits.store(z.to_bits(), Ordering::Relaxed);
        self.hw_forecast_bits
            .store(forecast.to_bits(), Ordering::Relaxed);
        self.forecast_ticks.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_entropy(&self, h: f64) {
        self.entropy_bits.store(h.to_bits(), Ordering::Relaxed);
        self.entropy_ticks.fetch_add(1, Ordering::Relaxed);
    }

    fn f64(bits: &AtomicU64) -> f64 {
        f64::from_bits(bits.load(Ordering::Relaxed))
    }

    pub fn get_batch_history(&self) -> Vec<BatchRecord> {
        // ponytail: Arc::try_unwrap would copy if we're the sole holder; in
        // practice the deque + last_batch always hold one ref each, so 2 copies.
        // Cheap (64 B/record × 80 entries).
        self.batch_history
            .lock()
            .map(|h| h.iter().map(|a| (**a).clone()).collect())
            .unwrap_or_default()
    }

    pub fn get_block_log(&self) -> Vec<BlockRecord> {
        self.block_log
            .lock()
            .map(|h| h.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn get_module_stats_data(
        &self,
        _uptime_secs: u64,
        ingested: u64,
        _channel_depth: usize,
        ips_tracked: usize,
        ram_bytes: usize,
        ram_limit_mb: usize,
    ) -> Vec<ModuleStats> {
        let elapsed = ((now_ms().saturating_sub(self.started_ms)) as f64 / 1000.0).max(0.001);
        let batches = self.batches_total.load(Ordering::Relaxed);

        let hw_rps = Metrics::f64(&self.hw_rps_bits);
        let hw_z = Metrics::f64(&self.hw_z_bits);
        let hw_f = Metrics::f64(&self.hw_forecast_bits);
        let entropy = Metrics::f64(&self.entropy_bits);

        let last_ev = self.last_batch_events.load(Ordering::Relaxed);

        let (_cpu_usage, total_system_memory_mb, _rss) = get_system_usage();

        vec![
            ModuleStats {
                label: "IPC".into(),
                events: self.requests_total.load(Ordering::Relaxed),
                errors: self.events_rejected.load(Ordering::Relaxed),
                rate_per_sec: self.requests_total.load(Ordering::Relaxed) as f64 / elapsed,
                detail: serde_json::json!({
                    "ingested": ingested,
                    "rejected": self.events_rejected.load(Ordering::Relaxed),
                }),
            },
            ModuleStats {
                label: "Detection".into(),
                events: ingested,
                errors: 0,
                rate_per_sec: ingested as f64 / elapsed,
                detail: serde_json::json!({
                    "batches": batches,
                    "promotions": self.promotions_total.load(Ordering::Relaxed),
                    "cold_skipped": self.cold_skipped_total.load(Ordering::Relaxed),
                    "blocks": self.blocks_detection.load(Ordering::Relaxed),
                    "subnet_blocks": self.blocks_subnet.load(Ordering::Relaxed),
                    "last_batch_events": last_ev,
                }),
            },
            ModuleStats {
                label: "Forecasting".into(),
                events: self.forecast_ticks.load(Ordering::Relaxed)
                    + self.entropy_ticks.load(Ordering::Relaxed),
                errors: 0,
                rate_per_sec: self.forecast_ticks.load(Ordering::Relaxed) as f64 / elapsed,
                detail: serde_json::json!({
                    "hw_rps": hw_rps,
                    "hw_forecast": hw_f,
                    "hw_zscore": hw_z,
                    "entropy": entropy,
                    "forecast_blocks": self.blocks_forecast.load(Ordering::Relaxed),
                    "forecast_ticks": self.forecast_ticks.load(Ordering::Relaxed),
                    "entropy_ticks": self.entropy_ticks.load(Ordering::Relaxed),
                }),
            },
            ModuleStats {
                label: "CGNAT".into(),
                events: self.cgnat_classify_ticks.load(Ordering::Relaxed),
                errors: 0,
                rate_per_sec: self.cgnat_classify_ticks.load(Ordering::Relaxed) as f64 / elapsed,
                detail: serde_json::json!({
                    "classify_ticks": self.cgnat_classify_ticks.load(Ordering::Relaxed),
                    "tier_allow": self.cgnat_tier_allow.load(Ordering::Relaxed),
                    "tier_challenge": self.cgnat_tier_challenge.load(Ordering::Relaxed),
                    "tier_powdrop": self.cgnat_tier_powdrop.load(Ordering::Relaxed),
                    "tier_block": self.cgnat_tier_block.load(Ordering::Relaxed),
                    "shm_publishes": self.shm_publish_count.load(Ordering::Relaxed),
                    "shm_cache_hits": self.shm_cache_hits.load(Ordering::Relaxed),
                }),
            },
            ModuleStats {
                label: "Analytics".into(),
                events: self.hll_insert_count.load(Ordering::Relaxed)
                    + self.cms_increment_count.load(Ordering::Relaxed),
                errors: 0,
                rate_per_sec: (self.hll_insert_count.load(Ordering::Relaxed)
                    + self.cms_increment_count.load(Ordering::Relaxed))
                    as f64
                    / elapsed,
                detail: serde_json::json!({
                    "hll_inserts": self.hll_insert_count.load(Ordering::Relaxed),
                    "cms_increments": self.cms_increment_count.load(Ordering::Relaxed),
                    "cms_decay_ticks": self.cms_decay_ticks.load(Ordering::Relaxed),
                }),
            },
            ModuleStats {
                label: "Mesh".into(),
                events: self.mesh_record_ban_count.load(Ordering::Relaxed),
                errors: 0,
                rate_per_sec: self.mesh_record_ban_count.load(Ordering::Relaxed) as f64 / elapsed,
                detail: serde_json::json!({
                    "record_bans": self.mesh_record_ban_count.load(Ordering::Relaxed),
                    "record_unbans": self.mesh_record_unban_count.load(Ordering::Relaxed),
                    "purge_ticks": self.mesh_purge_ticks.load(Ordering::Relaxed),
                    "hlc_ticks": self.mesh_hlc_ticks.load(Ordering::Relaxed),
                }),
            },
            ModuleStats {
                label: "Storage".into(),
                events: ips_tracked as u64,
                errors: 0,
                rate_per_sec: 0.0,
                detail: serde_json::json!({
                    "ram_mb": ram_bytes as f64 / (1024.0 * 1024.0),
                    "limit_mb": ram_limit_mb,
                    "ips_tracked": ips_tracked,
                    "total_system_memory_mb": total_system_memory_mb,
                }),
            },
        ]
    }
}

impl Metrics {
    /// Render all counters in Prometheus exposition format.
    /// ponytail: replaced dead stdout printer; upgrade path is
    /// metrics-exporter-prometheus if cardinality ever demands it.
    pub fn render_prometheus(&self) -> String {
        let elapsed = ((now_ms().saturating_sub(self.started_ms)) as f64 / 1000.0).max(0.001);

        macro_rules! emit {
            ($name:literal, $val:expr, $help:literal, $typ:literal) => {{
                let mut out = String::new();
                out.push_str(&format!("# HELP {} {}\n", $name, $help));
                out.push_str(&format!("# TYPE {} {}\n", $name, $typ));
                out.push_str(&format!("{} {}\n", $name, $val));
                out
            }};
        }

        let mut out = String::new();

        out.push_str(&emit!(
            "ramshield_uptime_seconds",
            elapsed as u64,
            "Process uptime in seconds.",
            "gauge"
        ));
        out.push_str(&emit!(
            "ramshield_requests_total",
            self.requests_total.load(Ordering::Relaxed),
            "Total IPC requests received.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_blocks_total",
            self.blocks_total.load(Ordering::Relaxed),
            "Total blocks issued.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_events_ingested_total",
            self.events_ingested.load(Ordering::Relaxed),
            "Total events ingested.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_frames_rejected_total",
            self.frames_rejected.load(Ordering::Relaxed),
            "IPC frames rejected at parse or auth-stripped decode.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_events_rejected_total",
            self.events_rejected.load(Ordering::Relaxed),
            "Total events rejected.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_events_shed_total",
            self.events_shed.load(Ordering::Relaxed),
            "Total low-signal events shed at the IPC high-water mark.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_ingest_channel_depth",
            self.ingest_channel_depth.load(Ordering::Relaxed),
            "Current buffered events in the bounded ingest queue.",
            "gauge"
        ));
        out.push_str(&emit!(
            "ramshield_active_cidr_blocks",
            self.active_cidr_blocks.load(Ordering::Relaxed),
            "Active IPv4/IPv6 CIDR prefixes applied to the kernel LPM trie (userspace mirror).",
            "gauge"
        ));
        out.push_str(&emit!(
            "ramshield_batches_total",
            self.batches_total.load(Ordering::Relaxed),
            "Total detection batches processed.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_promotions_total",
            self.promotions_total.load(Ordering::Relaxed),
            "Total IPs promoted to tracking.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_cold_skipped_total",
            self.cold_skipped_total.load(Ordering::Relaxed),
            "Total cold IPs skipped.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_blocks_detection",
            self.blocks_detection.load(Ordering::Relaxed),
            "Blocks from detection engine.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_blocks_subnet",
            self.blocks_subnet.load(Ordering::Relaxed),
            "Blocks from subnet module.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_enforcement_dropped_total",
            self.enforcement_dropped.load(Ordering::Relaxed),
            "Block commands dropped on a full detection-to-enforcement channel (attacker unblocked in kernel).",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_blocks_forecast",
            self.blocks_forecast.load(Ordering::Relaxed),
            "Blocks from forecasting.",
            "counter"
        ));
        // Patch A: bloom is an advisory revisit cache over PROMOTED ips (not
        // a block list). n = distinct promotes this 8s epoch, m = capacity in
        // bits. FP is estimated with k=2 (two hash slots per insert):
        //   p = (1 - e^(-2n/m))^2
        // Reported in ppm because at production n/m the useful values are
        // tiny; a percentage would round to 0.000.
        let bloom_bits = self.bloom_bits.load(Ordering::Relaxed);
        let bloom_n = self.bloom_inserts_epoch.load(Ordering::Relaxed);
        let bloom_fp_ppm = if bloom_bits == 0 {
            0u64
        } else {
            let ratio = 2.0 * bloom_n as f64 / bloom_bits as f64;
            let p = (1.0 - (-ratio).exp()).powi(2);
            (p * 1_000_000.0) as u64
        };
        out.push_str(&emit!(
            "ramshield_bloom_bits",
            bloom_bits,
            "Bloom filter capacity in bits (m).",
            "gauge"
        ));
        out.push_str(&emit!(
            "ramshield_bloom_inserts_epoch",
            bloom_n,
            "Distinct promoted IPs inserted into the bloom since the last clear (n).",
            "gauge"
        ));
        out.push_str(&emit!(
            "ramshield_bloom_clears_total",
            self.bloom_clears_total.load(Ordering::Relaxed),
            "Bloom epoch clears (advisory cache resets).",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_bloom_fp_ppm",
            bloom_fp_ppm,
            "Estimated bloom false-positive rate in parts per million (k=2).",
            "gauge"
        ));
        out.push_str(&emit!(
            "ramshield_forecast_ticks",
            self.forecast_ticks.load(Ordering::Relaxed),
            "Forecast ticks executed.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_entropy_ticks",
            self.entropy_ticks.load(Ordering::Relaxed),
            "Entropy check ticks executed.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_hw_rps",
            Metrics::f64(&self.hw_rps_bits),
            "Holt-Winters RPS forecast.",
            "gauge"
        ));
        out.push_str(&emit!(
            "ramshield_hw_zscore",
            Metrics::f64(&self.hw_z_bits),
            "Holt-Winters z-score.",
            "gauge"
        ));
        out.push_str(&emit!(
            "ramshield_hw_forecast",
            Metrics::f64(&self.hw_forecast_bits),
            "Holt-Winters forecast value.",
            "gauge"
        ));
        out.push_str(&emit!(
            "ramshield_entropy",
            Metrics::f64(&self.entropy_bits),
            "Current IP entropy.",
            "gauge"
        ));
        out.push_str(&emit!(
            "ramshield_cgnat_classify_ticks",
            self.cgnat_classify_ticks.load(Ordering::Relaxed),
            "CGNAT graduated classification cycles.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_cgnat_tier_allow",
            self.cgnat_tier_allow.load(Ordering::Relaxed),
            "CGNAT allow-tier verdicts.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_cgnat_tier_challenge",
            self.cgnat_tier_challenge.load(Ordering::Relaxed),
            "CGNAT challenge-tier (429+JS) verdicts.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_cgnat_tier_powdrop",
            self.cgnat_tier_powdrop.load(Ordering::Relaxed),
            "CGNAT proof-of-work drop verdicts.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_cgnat_tier_block",
            self.cgnat_tier_block.load(Ordering::Relaxed),
            "CGNAT hard-block verdicts.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_shm_publish_total",
            self.shm_publish_count.load(Ordering::Relaxed),
            "SHM rule-table publishes.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_shm_lookup_total",
            self.shm_lookup_count.load(Ordering::Relaxed),
            "SHM rule-table lookups.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_shm_cache_hits",
            self.shm_cache_hits.load(Ordering::Relaxed),
            "SHM rule-table lookups hitting an active rule.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_hll_insert_total",
            self.hll_insert_count.load(Ordering::Relaxed),
            "HLL cardinality-sketch inserts.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_cms_increment_total",
            self.cms_increment_count.load(Ordering::Relaxed),
            "Count-min sketch increments.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_cms_decay_ticks",
            self.cms_decay_ticks.load(Ordering::Relaxed),
            "Count-min sketch decay sweeps.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_mesh_record_ban_total",
            self.mesh_record_ban_count.load(Ordering::Relaxed),
            "Mesh CRDT ban records.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_mesh_record_unban_total",
            self.mesh_record_unban_count.load(Ordering::Relaxed),
            "Mesh CRDT unban records.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_mesh_purge_ticks",
            self.mesh_purge_ticks.load(Ordering::Relaxed),
            "Mesh CRDT TTL-purge sweeps.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_mesh_hlc_ticks",
            self.mesh_hlc_ticks.load(Ordering::Relaxed),
            "Mesh HLC logical-clock ticks.",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_xdp_v4_drops",
            self.xdp_v4_drops.load(Ordering::Relaxed),
            "XDP kernel IPv4 drops (COUNTERS PerCpuArray).",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_xdp_v6_drops",
            self.xdp_v6_drops.load(Ordering::Relaxed),
            "XDP kernel IPv6 drops (COUNTERS PerCpuArray).",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_xdp_wire_pass",
            self.xdp_wire_pass.load(Ordering::Relaxed),
            "XDP kernel wire passes (COUNTERS PerCpuArray).",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_wal_lsn",
            self.wal_lsn.load(Ordering::Relaxed),
            "Last committed WAL sequence number.",
            "gauge"
        ));
        out.push_str(&emit!(
            "ramshield_pending_expirations",
            self.pending_expirations.load(Ordering::Relaxed),
            "Pending TTL expirations in enforcement ring.",
            "gauge"
        ));
        out.push_str(&emit!(
            "ramshield_xdp_parse_fails",
            self.xdp_parse_fails.load(Ordering::Relaxed),
            "XDP kernel parse failures (COUNTERS PerCpuArray).",
            "counter"
        ));
        out.push_str(&emit!(
            "ramshield_xdp_apply_failures_total",
            self.xdp_apply_failures.load(Ordering::Relaxed),
            "Block/unblock decisions that failed to reach the kernel; the wire keeps passing the target while userspace+ WAL believe it is blocked (CIDR trie full / XDP failure).",
            "counter"
        ));
        // No trailing println! here — every emit stanza already ends in '\n',
        // and stdout writes from a render fn were a stray-syscall bug (2026-09).
        out
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    /// Cached /metrics text (1s TTL): counters move on a second grain, so
    /// re-rendering 18 stanzas per scrape is pure waste. Staleness ≤ 1s is
    /// the documented Prometheus compromise (same trade as get_system_usage).
    pub fn render_prometheus_cached(&self) -> Arc<str> {
        let mut cache = self
            .metrics_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((at, text)) = &*cache
            && at.elapsed() < std::time::Duration::from_secs(1)
        {
            return Arc::clone(text);
        }
        let text: Arc<str> = self.render_prometheus().into();
        *cache = Some((std::time::Instant::now(), Arc::clone(&text)));
        text
    }

    /// Pre-serialized dashboard JSON, invalidated by write-path sequence
    /// bumps. A 2s poll with no new blocks costs one Arc clone instead of
    /// ~3k String clones + a serde pass. Worst case a poll is one record stale.
    pub fn get_block_log_json(&self) -> Arc<str> {
        self.seq_cached_json(&self.block_seq, &self.blocks_json_cache, || {
            serde_json::to_string(&self.get_block_log()).unwrap_or_default()
        })
    }

    pub fn get_batch_history_json(&self) -> Arc<str> {
        self.seq_cached_json(&self.batch_seq, &self.batches_json_cache, || {
            serde_json::to_string(&self.get_batch_history()).unwrap_or_default()
        })
    }

    fn seq_cached_json(
        &self,
        seq: &AtomicU64,
        cache: &Mutex<Option<(u64, Arc<str>)>>,
        render: impl Fn() -> String,
    ) -> Arc<str> {
        // Invariant: a cached pair is stamped with a seq read BEFORE its
        // render. Stamping after would let a write racing mid-render hide
        // behind stale text forever. A raced render simply goes unpublished
        // (seq moved -> skip store); a later reader re-renders. Overserving
        // (text newer than stamp) only costs one extra render, never staleness.
        let want = seq.load(Ordering::Acquire);
        {
            let guard = cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((s, text)) = &*guard
                && *s == want
            {
                return Arc::clone(text);
            }
        }
        let text: Arc<str> = render().into();
        if seq.load(Ordering::Acquire) == want {
            *cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some((want, Arc::clone(&text)));
        }
        text
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn block_log_json_invalidates_on_new_block() {
        let m = Metrics::new();
        let a = m.get_block_log_json();
        assert_eq!(a.as_ref(), "[]");
        // No writes: cache hit — same Arc, zero re-render.
        let b = m.get_block_log_json();
        assert!(Arc::ptr_eq(&a, &b), "unchanged seq must reuse cached Arc");
        // A write bumps seq: next read must reflect it (no stale text).
        m.record_block_ip(&"1.2.3.4".parse().unwrap(), "flood", "detection");
        let c = m.get_block_log_json();
        assert!(!Arc::ptr_eq(&b, &c), "record_block must invalidate cache");
        assert!(c.contains("1.2.3.4"));
        // And re-caches at the new seq.
        let d = m.get_block_log_json();
        assert!(Arc::ptr_eq(&c, &d));
        let parsed: serde_json::Value = serde_json::from_str(&d).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 1);
    }

    #[test]
    fn batch_history_json_invalidates_on_new_batch() {
        let m = Metrics::new();
        let a = m.get_batch_history_json();
        let b = m.get_batch_history_json();
        assert!(Arc::ptr_eq(&a, &b));
        m.record_batch(BatchRecord {
            ts_ms: now_ms(),
            events: 10,
            unique_ips: 3,
            promoted: 1,
            cold_skipped: 2,
            promoted_events: 8,
            cold_skipped_events: 2,
            blocks: 0,
            hot_subnets: 0,
        });
        let c = m.get_batch_history_json();
        assert!(!Arc::ptr_eq(&a, &c));
        assert!(c.contains("\"events\":10"));
    }

    #[test]
    fn prometheus_cache_reuses_within_ttl() {
        let m = Metrics::new();
        let a = m.render_prometheus_cached();
        let b = m.render_prometheus_cached();
        assert!(
            Arc::ptr_eq(&a, &b),
            "second scrape within 1s must not re-render"
        );
        assert!(a.contains("ramshield_"));
        // Item 15 regression: the rendered text is the whole output — no
        // stray stdout writes happened (println! would not appear here, but
        // the old bug also added nothing to `out`; assert clean termination).
        assert!(a.ends_with('\n') && !a.ends_with("\n\n"));
    }

    #[test]
    fn prometheus_renders_ingest_gauges_with_writers() {
        let m = Metrics::new();
        m.set_channel_depth(12_345);
        m.set_active_cidr_blocks(7);
        let text = m.render_prometheus();
        assert!(text.contains("ramshield_ingest_channel_depth 12345"));
        assert!(text.contains("# TYPE ramshield_ingest_channel_depth gauge"));
        assert!(text.contains("ramshield_active_cidr_blocks 7"));
        assert!(text.contains("# TYPE ramshield_active_cidr_blocks gauge"));
    }

    /// Patch A: the bloom gauges must exist and be wired to their writers.
    /// Series absent = operators are blind to bloom fill, which is the whole
    /// point of the patch.
    #[test]
    fn prometheus_renders_bloom_series() {
        let m = Metrics::new();
        m.set_bloom_bits(8_000_000);
        m.record_bloom_inserts(1_000);
        let text = m.render_prometheus();
        for name in [
            "ramshield_bloom_bits",
            "ramshield_bloom_inserts_epoch",
            "ramshield_bloom_clears_total",
            "ramshield_bloom_fp_ppm",
        ] {
            assert!(
                text.contains(&format!("# TYPE {name}")),
                "missing Prometheus series {name}"
            );
        }
        assert!(text.contains("ramshield_bloom_bits 8000000"));
        assert!(text.contains("ramshield_bloom_inserts_epoch 1000"));
    }

    /// Patch A: FP estimate is k=2 -> p = (1 - e^(-2n/m))^2, in ppm.
    /// Pins the arithmetic: a wrong exponent silently reports a plausible
    /// but meaningless number.
    #[test]
    fn bloom_fp_ppm_matches_k2_formula() {
        let m = Metrics::new();
        m.set_bloom_bits(1_000_000);
        m.record_bloom_inserts(10_000);
        let text = m.render_prometheus();
        let got: f64 = text
            .lines()
            .find_map(|l| l.strip_prefix("ramshield_bloom_fp_ppm "))
            .and_then(|v| v.trim().parse().ok())
            .expect("bloom_fp_ppm must be present and numeric");
        let ratio: f64 = 2.0 * 10_000.0 / 1_000_000.0;
        let want = (1.0 - (-ratio).exp()).powi(2) * 1_000_000.0;
        assert!(
            (got - want).abs() <= 1.0,
            "fp_ppm {got} must match k=2 formula {want}"
        );
    }

    /// Patch A: an epoch clear zeroes n and therefore fp_ppm, and bumps the
    /// clear counter. Without the reset the gauge reports the peak fill of a
    /// filter that no longer holds those entries — a permanent false alarm.
    #[test]
    fn bloom_epoch_clear_resets_n_and_fp() {
        let m = Metrics::new();
        m.set_bloom_bits(8_000_000);
        m.record_bloom_inserts(50_000);
        assert!(
            m.render_prometheus()
                .contains("ramshield_bloom_inserts_epoch 50000")
        );
        m.bloom_epoch_clear();
        let text = m.render_prometheus();
        assert!(
            text.contains("ramshield_bloom_inserts_epoch 0"),
            "clear must zero the epoch insert count"
        );
        assert!(
            text.contains("ramshield_bloom_fp_ppm 0"),
            "fp must return to 0 after clear"
        );
        assert!(
            text.contains("ramshield_bloom_clears_total 1"),
            "clear counter must increment"
        );
    }

    /// Patch A: an unset bloom capacity must not divide by zero or emit NaN.
    #[test]
    fn bloom_fp_ppm_is_zero_when_capacity_unset() {
        let m = Metrics::new();
        m.record_bloom_inserts(1_000);
        let text = m.render_prometheus();
        assert!(
            text.contains("ramshield_bloom_fp_ppm 0"),
            "no capacity => no denominator => report 0, never NaN"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforcement_drops_counter_is_exported() {
        let m = Metrics::new();
        m.inc_enforcement_dropped();
        m.inc_enforcement_dropped();
        assert_eq!(m.enforcement_dropped.load(Ordering::Relaxed), 2);
        let out = m.render_prometheus();
        assert!(
            out.contains("ramshield_enforcement_dropped_total 2"),
            "enforcement drops must be scrapeable — a silently dropped security \
             command is otherwise invisible to the operator"
        );
    }

    #[test]
    fn xdp_apply_failures_counter_is_exported() {
        // The mirror of enforcement_dropped for the dataplane leg: when the
        // CIDR LPM trie fills (no LRU, hard cap) a subnet block silently stops
        // reaching the wire. This counter is the only scrapeable signal that
        // happened.
        let m = Metrics::new();
        m.inc_xdp_apply_failures();
        m.inc_xdp_apply_failures();
        m.inc_xdp_apply_failures();
        assert_eq!(m.xdp_apply_failures.load(Ordering::Relaxed), 3);
        let out = m.render_prometheus();
        assert!(
            out.contains("ramshield_xdp_apply_failures_total 3"),
            "XDP apply failures must be scrapeable — a silently full CIDR trie \
             otherwise looks like the subnet mitigation is simply not firing"
        );
    }

    #[test]
    fn block_log_evicts_at_configured_cap() {
        let m = Metrics::with_block_log(5);
        for i in 0..12 {
            m.record_block(&format!("10.0.0.{i}"), "high_rps", "detection");
        }
        let log = m.block_log.lock().unwrap();
        assert_eq!(log.len(), 5, "ring must evict oldest beyond cap");
        // newest survives, oldest gone
        assert_eq!(log.back().unwrap().ip, "10.0.0.11");
        assert_eq!(log.front().unwrap().ip, "10.0.0.7");
    }

    #[test]
    fn block_log_cap_floor_is_one() {
        // zero/nonsense config must not produce a zero-capacity deadlock ring
        let m = Metrics::with_block_log(0);
        m.record_block("10.0.0.1", "high_rps", "detection");
        assert_eq!(m.block_log.lock().unwrap().len(), 1);
    }
}
