use super::*;

impl Metrics {
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

    pub(crate) fn f64(bits: &AtomicU64) -> f64 {
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
