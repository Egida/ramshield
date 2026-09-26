use arc_swap::ArcSwap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{mpsc, watch};
use tracing::info;

use crate::config::Config;
use crate::detection::DetectionEngine;
use crate::enforcement::{EnforcementService, StubXdpApplier, XdpApplier};
use crate::forecasting::Forecaster;
use crate::metrics::{
    BatchRecord, BlockRecord, DashboardSnapshot, Metrics, ModuleStats, SubnetRow,
};
use crate::storage::Store;
use ramshield_storage::wal::Wal;
use ramshield_types::EnforceCommand;

pub struct Engine {
    pub config: Arc<arc_swap::ArcSwap<Config>>,
    pub store: Arc<Store>,
    pub metrics: Arc<Metrics>,
    shutdown: Arc<AtomicBool>,
    enforcement_tx: mpsc::Sender<EnforceCommand>,
    enforcement_rx: std::sync::Mutex<Option<mpsc::Receiver<EnforceCommand>>>,
    /// True only when the kernel XDP dataplane is loaded and attached. False
    /// for StubXdpApplier (degraded mode: in-band enforcement only). Read by
    /// `dashboard_snapshot()` so the UI can surface a "XDP inactive" chip.
    xdp_active: Arc<AtomicBool>,
    /// False until `boot_pipeline` returns Ok. Process-alive ≠ pipeline-ready.
    pipeline_ready: Arc<AtomicBool>,
    /// Set when `boot_pipeline` returns Err. `/healthz` stays 503.
    pipeline_failed: Arc<AtomicBool>,
    /// Shared depth counter for IPC event channel.
    /// Watch channel for async shutdown signaling (replaces AtomicBool polling).
    shutdown_tx: watch::Sender<bool>,
    /// F9: set by boot_pipeline so main can JOIN the batch/subnet threads
    /// (each final-flushes pre_aggs on exit) instead of sleeping blind 5s.
    detection: std::sync::Mutex<Option<Arc<crate::detection::DetectionEngine>>>,
}

impl Engine {
    pub fn new(cfg: Config, store: Arc<Store>, metrics: Arc<Metrics>) -> Self {
        let (enforcement_tx, enforcement_rx) = mpsc::channel(8192);
        let (shutdown_tx, _) = watch::channel(false);
        Self {
            config: Arc::new(ArcSwap::from_pointee(cfg)),
            store,
            metrics,
            shutdown: Arc::new(AtomicBool::new(false)),
            detection: std::sync::Mutex::new(None),
            enforcement_tx,
            enforcement_rx: std::sync::Mutex::new(Some(enforcement_rx)),
            xdp_active: Arc::new(AtomicBool::new(false)),
            pipeline_ready: Arc::new(AtomicBool::new(false)),
            pipeline_failed: Arc::new(AtomicBool::new(false)),
            shutdown_tx,
        }
    }

    #[deprecated(
        since = "0.2.0",
        note = "no-op stub; use start_async() instead to actually boot the pipeline"
    )]
    pub fn start(&self) {
        info!("Engine::start: sync stub — call start_async to actually boot");
    }

    /// Boot the full pipeline: store, detection, forecasting, IPC server.
    pub fn start_async(self: Arc<Self>) -> std::io::Result<std::thread::JoinHandle<()>> {
        let _cfg = self.config.load();
        std::thread::Builder::new()
            .name("rs-engine".into())
            .spawn(move || {
                // Multi-thread: IPC accept loop serves every connection's
                // read/write on this runtime; current_thread starved under
                // attack load (5s read timeouts during subnet floods).
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .enable_io()
                    .enable_time()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!("engine rt: {}", e);
                        return;
                    }
                };
                rt.block_on(async move {
                    match boot_pipeline(self.clone()).await {
                        Ok(()) => {
                            self.pipeline_ready.store(true, Ordering::Release);
                        }
                        Err(e) => {
                            tracing::error!("pipeline: {}", e);
                            self.pipeline_failed.store(true, Ordering::Release);
                        }
                    }
                });
            })
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.shutdown_tx.send(true);
    }

    pub fn shutdown_rx(&self) -> watch::Receiver<bool> {
        self.shutdown_tx.subscribe()
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    /// Tests construct Engine without boot_pipeline. Production never calls this.
    #[cfg(test)]
    pub fn mark_pipeline_ready_for_test(&self) {
        self.pipeline_ready.store(true, Ordering::Release);
    }

    /// F9: join detection batch/subnet threads with a grace cap. Call after
    /// shutdown() — replaces the fixed sleep in main.
    pub fn join_workers(&self, grace: std::time::Duration) {
        // no-unwrap gate (CI lint-no-unwrap scans src/): poisoning must not
        // abort shutdown — a panicked holder still left a valid Option<Arc>
        // behind, and join is exactly what shutdown needs to do then.
        if let Some(det) = self
            .detection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            det.join_workers(grace);
        }
    }

    pub fn dashboard_snapshot(&self) -> DashboardSnapshot {
        let store = &self.store;
        let metrics = &self.metrics;
        let stats = store.get_stats();
        let (cpu_usage, total_ram_mb, memory_usage_mb) = crate::metrics::get_system_usage();

        let ram_pct = if stats.ram_limit_mb > 0 {
            (stats.ram_bytes as f64 / (stats.ram_limit_mb as f64 * 1048576.0) * 100.0).min(100.0)
        } else {
            0.0
        };
        let ingested = metrics.events_ingested.load(Ordering::Relaxed);
        let batches = metrics.batches_total.load(Ordering::Relaxed);
        let promotions = metrics.promotions_total.load(Ordering::Relaxed);
        let blocks_applied = metrics.blocks_detection.load(Ordering::Relaxed)
            + metrics.blocks_subnet.load(Ordering::Relaxed)
            + metrics.blocks_forecast.load(Ordering::Relaxed);
        let channel_depth = self
            .detection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map_or(0, |d| d.event_queue_depth());
        metrics.set_channel_depth(channel_depth);

        DashboardSnapshot {
            ts_ms: crate::metrics::now_ms(),
            uptime_secs: stats.uptime_secs,
            ips_tracked: stats.ips_tracked,
            blocked_total: stats.blocked,
            ram_bytes: stats.ram_bytes,
            ram_limit_mb: stats.ram_limit_mb,
            ram_pct,
            cpu_usage,
            memory_usage_mb,
            total_ram_mb,
            ipc_requests: metrics.requests_total.load(Ordering::Relaxed),
            events_ingested: ingested,
            events_rejected: metrics.events_rejected.load(Ordering::Relaxed),
            frames_rejected_total: metrics.frames_rejected.load(Ordering::Relaxed),
            channel_depth,
            events_shed: metrics.events_shed.load(Ordering::Relaxed),
            batches_total: batches,
            promotions,
            cold_skipped: metrics.cold_skipped_total.load(Ordering::Relaxed),
            blocks_applied,
            pipeline: crate::metrics::PipelineFlow {
                ingest: ingested,
                queued: channel_depth as u64,
                batched: batches,
                promoted: promotions,
                merged: stats.ips_tracked as u64,
                blocked: blocks_applied,
            },
            wal_lsn: self.metrics.wal_lsn.load(Ordering::Relaxed),
            pending_expirations: self.metrics.pending_expirations.load(Ordering::Relaxed),
            is_healthy: !self.is_shutting_down()
                && ram_pct < 95.0
                && self.pipeline_ready.load(Ordering::Acquire)
                && !self.pipeline_failed.load(Ordering::Acquire),
            health_reason: if self.is_shutting_down() {
                "shutting down".into()
            } else if self.pipeline_failed.load(Ordering::Acquire) {
                "pipeline failed".into()
            } else if !self.pipeline_ready.load(Ordering::Acquire) {
                "starting".into()
            } else if ram_pct >= 95.0 {
                "ram pressure".into()
            } else {
                "running".into()
            },
            xdp_active: self.xdp_active.load(Ordering::Acquire),
        }
    }

    pub fn get_batch_history(&self) -> Vec<BatchRecord> {
        self.metrics.get_batch_history()
    }

    /// Pre-serialized JSON variants for the dashboard polling endpoints
    /// (RAM-for-CPU item 16): unchanged data costs one Arc clone per poll.
    pub fn get_batch_history_json(&self) -> std::sync::Arc<str> {
        self.metrics.get_batch_history_json()
    }

    pub fn get_block_log_json(&self) -> std::sync::Arc<str> {
        self.metrics.get_block_log_json()
    }

    pub fn get_block_log(&self) -> Vec<BlockRecord> {
        self.metrics.get_block_log()
    }

    pub fn get_active_blocks(&self) -> Vec<BlockRecord> {
        let mut blocks: Vec<BlockRecord> = self
            .store
            .get_all_blocked_ips()
            .into_iter()
            .filter_map(|ip| {
                let value = self.store.get(&ip)?;
                let ramshield_storage::Value::IpRecord(record) = value else {
                    return None;
                };
                let ramshield_storage::BlockState::Blocked {
                    ref reason,
                    since_ns,
                } = record.block_state
                else {
                    return None;
                };
                Some(BlockRecord {
                    ts_ms: since_ns / 1_000_000,
                    ip: ip.to_string(),
                    reason: reason.as_str().to_string(),
                    module: "enforcement".to_string(),
                })
            })
            .collect();
        blocks.sort_unstable_by_key(|block| std::cmp::Reverse(block.ts_ms));
        blocks
    }

    pub fn get_hot_subnets(&self) -> Vec<SubnetRow> {
        // ponytail: select_nth_unstable finds the 100th in O(n) — old sort was
        // O(n log n) when only top-100 is kept. Strings still allocate; the
        // real win is avoiding the sort.
        if self.store.subnet_table().is_empty() {
            return Vec::new();
        }
        let mut rows: Vec<SubnetRow> = self
            .store
            .subnet_table()
            .iter()
            .map(|e| {
                let rec = e.value();
                // Task 1: IpNetwork Display = family-complete CIDR
                // ("198.51.100.0/24" / "2001:db8::/64"); the old hand-rolled
                // "{}.{}.{}" rendered v6 as garbage and had no /24 suffix.
                SubnetRow {
                    prefix: rec.network.to_string(),
                    events: rec.total_rps,
                }
            })
            .collect();
        if rows.len() > 100 {
            rows.select_nth_unstable_by_key(100, |r| std::cmp::Reverse(r.events));
            rows.truncate(100);
        } else {
            rows.sort_by_key(|r| std::cmp::Reverse(r.events));
        }
        rows
    }

    pub fn get_module_stats(&self) -> Vec<ModuleStats> {
        let stats = self.store.get_stats();
        let ingested = self.metrics.events_ingested.load(Ordering::Relaxed);
        let channel_depth = self
            .detection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map_or(0, |d| d.event_queue_depth());
        self.metrics.set_channel_depth(channel_depth);
        self.metrics.get_module_stats_data(
            stats.uptime_secs,
            ingested,
            channel_depth,
            stats.ips_tracked,
            stats.ram_bytes,
            stats.ram_limit_mb,
        )
    }

    /// Send a manual mitigation command from the dashboard console.
    /// Uses try_send to avoid blocking on a full enforcement channel.
    pub fn send_mitigation(&self, cmd: EnforceCommand) -> Result<(), String> {
        self.enforcement_tx
            .try_send(cmd)
            .map_err(|e| format!("enforcement channel full or closed: {}", e))
    }
}

/// Capability preflight for the XDP path.
///
/// BPF map creation fails with EPERM (surfaced by aya as "failed to create
/// map") when the process lacks CAP_BPF/CAP_NET_ADMIN. File capabilities live
/// on the inode, so every `cargo build` silently drops them — the failure then
/// looks like a map bug. Read CapEff and hand back the exact setcap command.
#[cfg(feature = "xdp")]
fn xdp_capability_hint() -> String {
    const CAP_NET_ADMIN: u64 = 12;
    const CAP_PERFMON: u64 = 38;
    const CAP_BPF: u64 = 39;
    let needed = [
        ("cap_net_admin", CAP_NET_ADMIN),
        ("cap_perfmon", CAP_PERFMON),
        ("cap_bpf", CAP_BPF),
    ];
    let eff = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("CapEff:"))
                .and_then(|l| u64::from_str_radix(l.split_whitespace().nth(1)?, 16).ok())
        })
        .unwrap_or(0);
    let missing: Vec<&str> = needed
        .iter()
        .filter(|(_, bit)| eff & (1u64 << bit) == 0)
        .map(|(name, _)| *name)
        .collect();
    if missing.is_empty() {
        return "capabilities present; check kernel BPF limits".to_string();
    }
    format!(
        "missing {} — re-apply after every build: sudo setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' target/release/ramshield",
        missing.join(",")
    )
}

async fn boot_pipeline(engine: Arc<Engine>) -> std::io::Result<()> {
    // FIX: use engine.config directly for live hot-reload — not a separate ArcSwap.
    // The old code did cfg_snapshot.clone().into_handle() which created a parallel
    // ArcSwap that never saw updates from api_set_config.
    let cfg_handle = engine.config.clone();

    // Boot-time snapshot: read-once values (XDP, WAL, forecaster config).
    // These are immutable once the pipeline starts; changing them requires restart.
    let cfg_arc = engine.config.load(); // Arc<Config>
    let cfg_snapshot = cfg_arc.as_ref().clone(); // owned Config clone

    // Use engine's shared store and metrics (shared with dashboard)
    let store = engine.store.clone();
    let metrics = engine.metrics.clone();

    // Take the enforcement receiver ONCE
    let enforcement_rx = engine
        .enforcement_rx
        .lock()
        .map_err(|_| std::io::Error::other("enforcement receiver lock poisoned"))?
        .take()
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "enforcement service already started",
            )
        })?;
    // The service follows the engine shutdown flag through a dedicated watcher.
    let enforcement_shutdown = Arc::new(AtomicBool::new(false));
    // Dataplane: real aya XDP when [xdp].enabled, else in-band-only stub.
    // Load failure is not fatal — daemon runs degraded (in-band enforcement
    // still works) and logs loudly. See plans/2026-08-22_enforcement-production.md.
    let xdp_box: Box<dyn XdpApplier> = if cfg_snapshot.xdp.enabled {
        #[cfg(feature = "xdp")]
        {
            let mut applier = crate::enforcement::xdp::AyaXdpApplier::new(
                &cfg_snapshot.xdp.interface,
                &cfg_snapshot.xdp.mode,
            );
            match applier.load_and_attach() {
                Ok(()) => {
                    tracing::info!(iface = %cfg_snapshot.xdp.interface, mode = %cfg_snapshot.xdp.mode, "XDP dataplane active");
                    engine.xdp_active.store(true, Ordering::Release);
                    Box::new(applier)
                }
                Err(e) => {
                    // A rebuild replaces the binary inode and drops its file
                    // capabilities, which the kernel reports as an opaque map
                    // creation failure. Name the real cause instead.
                    tracing::error!(
                        iface = %cfg_snapshot.xdp.interface,
                        mode = %cfg_snapshot.xdp.mode,
                        error = %e,
                        remediation = %xdp_capability_hint(),
                        "XDP load/attach failed — falling back to in-band enforcement"
                    );
                    Box::new(StubXdpApplier)
                }
            }
        }
        #[cfg(not(feature = "xdp"))]
        {
            tracing::warn!(
                "[xdp].enabled=true but binary built without 'xdp' feature — in-band enforcement only"
            );
            Box::new(StubXdpApplier)
        }
    } else {
        Box::new(StubXdpApplier)
    };
    let mut enforcement = EnforcementService::new(
        store.clone(),
        metrics.clone(),
        xdp_box,
        enforcement_shutdown.clone(),
    );
    // Crash-durable block state: open WAL, replay live blocks into the store
    // BEFORE run() reconciles store → XDP.
    if cfg_snapshot.wal.enabled {
        match Wal::open(
            &cfg_snapshot.wal.dir,
            cfg_snapshot.wal.compress,
            cfg_snapshot.wal.durability,
            cfg_snapshot.wal.seg_max_bytes,
            cfg_snapshot.wal.retention_max_bytes,
        ) {
            Ok(wal) => {
                let wal = Arc::new(wal);
                match ramshield_enforcement::replay_wal_into_store(&store, &wal) {
                    Ok(pairs) => {
                        tracing::info!(
                            "WAL enabled at {} — {} blocks restored ({} with TTL)",
                            cfg_snapshot.wal.dir,
                            pairs.len(),
                            pairs.iter().filter(|(_, t)| *t > 0).count()
                        );
                        // P1-4: restored blocks must expire on schedule — re-arm
                        // the TTL ring (expirations/buckets are empty at boot).
                        enforcement.restore_expirations(pairs);
                        match ramshield_enforcement::replay_wal_cidrs(&wal) {
                            Ok(cidrs) => enforcement.restore_cidr_blocks(cidrs),
                            Err(e) => tracing::error!("WAL CIDR replay failed: {}", e),
                        }
                    }
                    Err(e) => {
                        tracing::error!("WAL replay failed: {} — starting with empty block set", e)
                    }
                }
                enforcement = enforcement.with_wal(wal);
            }
            Err(e) => {
                tracing::error!(
                    "WAL open failed ({}): {} — running without durability",
                    cfg_snapshot.wal.dir,
                    e
                );
            }
        }
    }
    let mut shutdown_rx = engine.shutdown_rx();
    let enforcement_handle = {
        let enforcement = enforcement;
        tokio::spawn(async move {
            if let Err(e) = enforcement.run(enforcement_rx).await {
                tracing::error!("enforcement service: {}", e);
            }
        })
    };

    let detection = Arc::new(DetectionEngine::try_new(
        store.clone(),
        cfg_handle.clone(),
        engine.enforcement_tx.clone(),
        metrics.clone(),
        engine.shutdown.clone(),
    )?);
    let event_tx = detection.event_sender();
    detection
        .clone()
        .spawn_workers(cfg_snapshot.engine.worker_threads);
    *engine.detection.lock().unwrap_or_else(|e| e.into_inner()) = Some(detection.clone());

    let mut forecaster = if cfg_snapshot.forecasting.enabled {
        let forecaster = Arc::new(Forecaster::new(
            store.clone(),
            cfg_snapshot.forecasting.clone(),
            engine.enforcement_tx.clone(),
            metrics.clone(),
        ));
        let fc = forecaster.clone();
        let mut fc_rx = engine.shutdown_rx();
        let handle = tokio::spawn(async move {
            tokio::select! {
                _ = fc.run() => {}
                _ = fc_rx.changed() => {
                    tracing::info!("forecaster: shutdown signal received");
                }
            }
        });
        Some((forecaster, handle))
    } else {
        // P1-9: honoring `forecasting.enabled` — previously the flag was
        // read nowhere and the span always ran (queue feed kept growing).
        tracing::info!(
            "forecasting.enabled=false — forecaster span not started (threat_sample queue not fed)"
        );
        None
    };

    let server = crate::ipc::server::IpcServer::bind(
        cfg_handle.clone(),
        engine.clone(),
        event_tx,
        store,
        engine.enforcement_tx.clone(),
    )
    .await?;
    // Pipeline is genuinely serving: enforcement, detection, IPC all running.
    // boot_pipeline blocks in select! below until shutdown, so this is the
    // one place readiness can be observed while the daemon is live.
    engine.pipeline_ready.store(true, Ordering::Release);

    // Graceful shutdown: wait for signal, then join tasks.
    tokio::select! {
        _ = server.start() => {}
        _ = shutdown_rx.changed() => {
            tracing::info!("pipeline: shutdown signal received, draining...");
            enforcement_shutdown.store(true, Ordering::Release);
            // Join with timeout to avoid hanging on stuck tasks.
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            tokio::select! {
                _ = enforcement_handle => {}
                _ = tokio::time::sleep_until(deadline) => {
                    tracing::warn!("enforcement shutdown timed out");
                }
            }
            if let Some((_, handle)) = forecaster.as_mut() {
                    tokio::select! {
                        _ = handle => {}
                        _ = tokio::time::sleep_until(deadline) => {
                            tracing::warn!("forecaster shutdown timed out");
                        }
                    }
                }
        }
    }
    Ok(())
}

#[cfg(test)]
mod startup_tests {
    //! BACKLOG #14 — engine startup integration tests.
    //! Lives in-tree (not `tests/`) so it can reach crate-private engine
    //! internals; rides `cargo test --lib`.
    use super::*;
    use crate::Config;
    use crate::metrics::Metrics;
    use crate::storage::Store;
    use std::sync::Arc;

    #[test]
    fn engine_constructs_with_default_config() {
        let _engine = Engine::new(
            Config::default(),
            Arc::new(Store::new(16)),
            Arc::new(Metrics::new()),
        );
    }

    #[test]
    fn engine_start_then_snapshot_default_state() {
        let engine = Engine::new(
            Config::default(),
            Arc::new(Store::new(16)),
            Arc::new(Metrics::new()),
        );
        #[allow(deprecated)]
        engine.start();
        engine.mark_pipeline_ready_for_test();
        let snap = engine.dashboard_snapshot();
        assert!(snap.is_healthy);
        assert_eq!(snap.ips_tracked, 0);
        assert_eq!(snap.blocked_total, 0);
        assert_eq!(snap.events_ingested, 0);
    }

    #[test]
    fn engine_module_stats_have_four_canonical_rows() {
        let engine = Engine::new(
            Config::default(),
            Arc::new(Store::new(16)),
            Arc::new(Metrics::new()),
        );
        #[allow(deprecated)]
        engine.start();
        let stats = engine.get_module_stats();
        assert_eq!(stats.len(), 7);
        let labels: Vec<&str> = stats.iter().map(|m| m.label.as_str()).collect();
        assert!(labels.contains(&"IPC"));
        assert!(labels.contains(&"Detection"));
        assert!(labels.contains(&"Forecasting"));
        assert!(labels.contains(&"Storage"));
    }

    #[test]
    fn engine_snapshot_unhealthy_when_shutting_down() {
        let engine = Engine::new(
            Config::default(),
            Arc::new(Store::new(16)),
            Arc::new(Metrics::new()),
        );
        engine.shutdown();
        let snap = engine.dashboard_snapshot();
        assert!(!snap.is_healthy);
        assert_eq!(snap.health_reason, "shutting down");
    }

    #[test]
    fn engine_snapshot_unhealthy_until_pipeline_ready() {
        let engine = Engine::new(
            Config::default(),
            Arc::new(Store::new(16)),
            Arc::new(Metrics::new()),
        );
        let snap = engine.dashboard_snapshot();
        assert!(!snap.is_healthy);
        assert_eq!(snap.health_reason, "starting");
        engine.mark_pipeline_ready_for_test();
        let snap = engine.dashboard_snapshot();
        assert!(snap.is_healthy);
        assert_eq!(snap.health_reason, "running");
        engine.pipeline_failed.store(true, Ordering::Release);
        let snap = engine.dashboard_snapshot();
        assert!(!snap.is_healthy);
        assert_eq!(snap.health_reason, "pipeline failed");
    }

    #[test]
    fn engine_snapshot_unhealthy_when_ram_pressure() {
        // RED: set ram_limit_mb=1 MB and ram_bytes = 1.5 MB → ram_pct > 95%.
        // Broken code (8c159cc): is_healthy stays true. Fixed code: flips to false.
        let store = Arc::new(Store::new(16));
        store.set_ram_limit_mb_for_testing(1);
        store.set_ram_bytes_for_testing(1_572_864); // 1.5 MB > 1 MB → ram_pct = 100.0
        let engine = Engine::new(Config::default(), store, Arc::new(Metrics::new()));
        engine.mark_pipeline_ready_for_test();
        let snap = engine.dashboard_snapshot();
        assert!(
            !snap.is_healthy,
            "is_healthy should flip false at ram_pct=100%"
        );
        assert_eq!(snap.health_reason, "ram pressure");
    }

    fn blocked_record(ip: std::net::IpAddr) -> crate::storage::IpRecord {
        use crate::BlockReason;
        use crate::storage::{BlockState, IpRecord};
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
                reason: BlockReason::HighRps,
                since_ns: 0,
            },
        }
    }

    #[test]
    fn active_blocks_emit_canonical_reason_tokens() {
        // RED: format!("{reason:?}") emitted "HighRps" from /status while every
        // other boundary emits the canonical token "high_rps" (BlockReason::as_str).
        let store = Arc::new(Store::new(16));
        store
            .insert(
                "10.9.9.9".parse().unwrap(),
                crate::storage::Value::IpRecord(blocked_record("10.9.9.9".parse().unwrap())),
                None,
                1 << 30,
            )
            .unwrap();
        let engine = Engine::new(Config::default(), store, Arc::new(Metrics::new()));
        let blocks = engine.get_active_blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].reason, "high_rps");
    }
}
