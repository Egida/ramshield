use super::*;

impl DetectionEngine {
    /// 2-probe fill estimate: FP ≈ (1 - e^(-2n/b))². At 2n/b = 0.1 the FP is
    /// ~1%; the guard fires at 2n/b = 0.25 (n ≥ b/20) — above that the
    /// cold-skip gate has stopped skipping and the store is about to bloat.
    /// Sizing rule: bloom_bits ≈ 20 × (promoted IPs per 8 s epoch).
    pub(crate) fn bloom_saturated(&self) -> bool {
        let bits = self.metrics.bloom_bits.load(Ordering::Relaxed);
        let n = self.metrics.bloom_inserts_epoch.load(Ordering::Relaxed);
        bits != 0 && n.saturating_mul(20) >= bits
    }

    /// Subnet-scale batch block — reads subnet_table only, not full store key scan.
    pub(crate) fn subnet_batch_loop(self: Arc<Self>) {
        let tick = std::time::Duration::from_millis(500);
        // Bloom clear cadence: 8s. Advisory cache resets faster than the slowest
        // legitimate IP's revisit window, so cold-skip stays effective.
        // Without clear, every bit becomes set within hours and cold-skip dies.
        let bloom_clear_ns: u64 = 8 * 1_000_000_000;
        let mut last_bloom_clear_ns = now_ns();
        // P1-6: periodic eviction of expired entries. Without this sweep,
        // expired entries (~136 B each) stay in DashMap indefinitely, counted
        // in ram_bytes, causing CapacityExceeded despite actual working-set
        // being well under the limit.
        let evict_interval_ns: u64 = 60 * 1_000_000_000;
        let mut last_evict_ns = now_ns();
        loop {
            if self.shutdown.load(Ordering::Acquire) {
                info!("Subnet batch loop shutting down");
                break;
            }
            std::thread::sleep(tick);
            // Saturation guard: a mis-sized bloom_bits (too small for the
            // promote rate) degrades the gate long before the 8 s boundary.
            // Self-heal: clear early; the warn carries the sizing rule so a
            // re-raise of detection.bloom_bits is the fix.
            if self.bloom_saturated() {
                let saturated_at = self.metrics.bloom_inserts_epoch.load(Ordering::Relaxed);
                let bits = self.config.load().detection.bloom_bits;
                self.bloom.store(Arc::new(BloomFilter::new(bits)));
                self.metrics.bloom_epoch_clear();
                self.metrics.record_bloom_saturation_clear();
                last_bloom_clear_ns = now_ns();
                warn!(
                    inserts_epoch = saturated_at,
                    bloom_bits = bits,
                    "bloom saturated before epoch end — early clear; sizing rule: \
                     detection.bloom_bits ≈ 20 × promoted IPs per 8 s"
                );
            }
            if now_ns().saturating_sub(last_bloom_clear_ns) >= bloom_clear_ns {
                // ponytail: atomic swap — no lock held during clear.
                // Old bloom is reclaimed when last reader releases its guard.
                let bits = self.config.load().detection.bloom_bits;
                self.bloom.store(Arc::new(BloomFilter::new(bits)));
                // Patch A: the epoch ended, so n resets and fp_ppm drops to
                // 0. Without this the gauge would report the peak fill of a
                // filter that no longer holds those entries — a permanent
                // false alarm.
                self.metrics.bloom_epoch_clear();
                last_bloom_clear_ns = now_ns();
            }
            // P1-6: sweep expired entries every 60s.
            if now_ns().saturating_sub(last_evict_ns) >= evict_interval_ns {
                let evicted = self.store.evict_expired();
                if evicted > 0 {
                    debug!("evict_expired: removed {} expired entries", evicted);
                }
                last_evict_ns = now_ns();
            }
            // P1-10 fix: prune subnet_table when > 100K entries. This must
            // run INSIDE the loop — the original placement after `break`
            // made it a shutdown-only statement, so the table grew without
            // bound under spoofed-subnet floods until the host OOMed. Runs
            // before the batch_block_enabled `continue` so pruning cannot
            // be skipped when batch blocking is off. DashMap::len() is a
            // per-shard sum — cheap enough for the 500ms tick.
            // P2 fix: scan before prune - this ensures freshly promoted subnets
            // from the current tick are evaluated for blocking before they
            // might be pruned in the following iteration. This fixes the
            // scenario where a new `/24` swarm meets the dual gate in the
            // current scan but would be evicted before the next scan.
            self.subnet_batch_scan();

            // After scanning, prune subnet_table for stale entries. This order
            // prevents active hot subnets from being incorrectly evicted
            // during the same scan cycle.
            let st = self.store.subnet_table();
            if st.len() > 100_000 {
                // P2 fix (F6): the old predicate (total_rps==0 &&
                // unique_ips()==0) is dead against the very attack that
                // motivated the prune — a spoofed-subnet flood sends one
                // burst per /24 then goes silent; total_rps only zeroes on a
                // NEW merge rollover >2s later, so flooded entries never
                // match and the table grows toward 16.7M /24s while every
                // 500ms full-iter (prune + hot()) slows down. Evict on
                // staleness instead: silent >8s = 4 gate-windows = functionally
                // zero for the 2s dual gate, so a live swarm can't be pruned.
                let now = now_ns();
                let stale_ns = 8_000_000_000;
                let mut candidates: Vec<_> = st
                    .iter()
                    .filter_map(|e| {
                        let r = e.value();
                        if now.saturating_sub(r.last_updated_ns) > stale_ns {
                            Some(*e.key())
                        } else {
                            None
                        }
                    })
                    .collect();
                candidates.truncate(st.len().saturating_sub(80_000)); // evict down to 80K
                for key in candidates {
                    st.remove(&key);
                }
            }
        }
    }

    /// One pass of the subnet dual gate + batch-block emit. Split out of
    /// `subnet_batch_loop` so tests can fire the scan deterministically.
    pub(crate) fn subnet_batch_scan(&self) {
        let cfg = self.config.load();
        if !cfg.detection.batch_block_enabled {
            return;
        }
        // Dual gate: unique-IP swarm signal AND raw event volume. Either
        // alone mis-fires (single flood IP trips volume; slow drip from
        // many IPs trips uniqueness).
        let ip_threshold = cfg.detection.subnet_batch_threshold as u64;
        let ev_threshold = cfg.detection.subnet_batch_min_events;

        let hot: Vec<(SubnetKey, u64, u64, IpNetwork)> = self
            .store
            .subnet_table()
            .iter()
            .filter_map(|e| {
                let r = e.value();
                // IPv6 plan Task 2 (G1): v4 uniques come from the 256-bit
                // host bitmap (free inside the iter shard lock); a v6 /64
                // has no bitmap — exact count is subnet_index cardinality
                // (separate DashMap, no lock nesting).
                let uniq = if r.network.family() == 4 {
                    r.unique_ips()
                } else {
                    // Windowed: the raw index counts LIFETIME members (never
                    // pruned on unblock) — a cooled /64 with 60 historical
                    // hosts plus a small fresh burst would pass the gate and
                    // batch-block ~55 innocent IPs (review P1-2).
                    let now = now_ns();
                    self.store
                        .subnet_member_count_windowed(*e.key(), SUBNET_WINDOW_NS, now)
                };
                if uniq >= ip_threshold && r.total_rps >= ev_threshold {
                    Some((*e.key(), uniq, r.total_rps, r.network))
                } else {
                    None
                }
            })
            .collect();

        for (sk, uniq, count, cidr) in hot {
            let mut rejected_q = 0u32;
            // Decision event, not an anomaly — state is live on the dashboard
            // (SSE subnet grid + block counters). Per-tick WARN for every hot
            // subnet floods logs under sustained flood (500/s); debug level.
            debug!(
                cidr = %cidr,
                unique_ips = uniq,
                events = count,
                "Batch block subnet in window"
            );
            debug!("Batch blocking subnet key {:#x}", sk);

            // The CIDR is one decision. Exact-IP expansion leaves rotating
            // hosts uncovered; the XDP LPM map enforces the complete prefix.
            let now = now_ns();
            let subnet_tier = self.cgnat_guard.classify_subnet(cidr.addr, uniq, count);
            if subnet_tier == ramshield_cgnat::CGNAT_TIER_BLOCK {
                // Same admission gate as per-IP blocks: one /24 decision
                // re-emitted every 500ms tick would otherwise re-do the
                // whole enforcement path for the prefix. Cooldown =
                // subnet_burst_ttl/2; rejection still retries next tick.
                let key = (cidr.addr, BlockReason::SubnetBatch);
                if self.admit_mitigation(key, cfg.detection.subnet_burst_ttl_secs, now) {
                    let cmd = EnforceCommand {
                        decision_id: Uuid::new_v4(),
                        policy_version: 1,
                        source: "detection".into(),
                        actor: "system".into(),
                        timestamp_utc: (now / 1_000_000_000) as i64,
                        ttl_seconds: cfg.detection.subnet_burst_ttl_secs,
                        reason: "subnet_burst".into(),
                        ip: cidr.addr,
                        cidr: Some(cidr),
                        action: EnforceAction::Block,
                    };
                    if self.enforcement_tx.try_send(cmd).is_err() {
                        self.retreat_mitigation(key);
                        rejected_q += 1;
                        self.metrics.inc_enforcement_dropped();
                        warn!(cidr = %cidr, rejected_q, "enforcement queue full; CIDR block rejected");
                    } else {
                        self.metrics.blocks_subnet.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }

            // One subnet decision, one owner. The SHM rule, CGNAT tier, and
            // history row are subnet-level facts — each emitted ONCE, not
            // once per member host. (The former member loop repeated all of
            // them N times with the same subnet key, so a single /24
            // decision reported N blocks and N history rows for hosts that
            // were never individually blocked.)
            self.metrics.inc_cgnat_classify();
            match subnet_tier {
                ramshield_cgnat::CGNAT_TIER_ALLOW => self.metrics.inc_cgnat_allow(),
                ramshield_cgnat::CGNAT_TIER_CHALLENGE => self.metrics.inc_cgnat_challenge(),
                ramshield_cgnat::CGNAT_TIER_XDP_DROP => self.metrics.inc_cgnat_powdrop(),
                ramshield_cgnat::CGNAT_TIER_BLOCK => self.metrics.inc_cgnat_block(),
                _ => {} // ponytail: future tiers — no crash, just don't count
            }
            let shm_key = match cidr.addr {
                std::net::IpAddr::V4(network) => {
                    ramshield_cgnat::shm::subnet_key(u32::from(network), cidr.prefix_len)
                }
                std::net::IpAddr::V6(_) => sk as u64,
            };
            if subnet_tier != ramshield_cgnat::CGNAT_TIER_ALLOW {
                if self.shm_table.publish_rule(
                    shm_key,
                    cfg.detection.subnet_burst_ttl_secs * 1000,
                    subnet_tier,
                    0,
                    true,
                ) {
                    self.metrics.inc_shm_publish();
                } else {
                    warn!(
                        subnet = ?cidr,
                        "CGNAT SHM probe window saturated; userspace enforcement remains authoritative"
                    );
                }
            }
            if subnet_tier == ramshield_cgnat::CGNAT_TIER_BLOCK {
                // The decision, in operator history — one row keyed by the
                // network address. blocks_subnet was already counted when the
                // enforcement command was accepted above (no double count).
                self.metrics
                    .record_block_ip(&cidr.addr, "subnet_batch", "detection");
                // Window consumed: this scan acted on the accumulated rate,
                // roll the window fresh. Only here — resetting on
                // ALLOW/CHALLENGE would zero total_rps every 500 ms tick and
                // make the 50k TIER_BLOCK rate floor unreachable for any
                // burst slower than 100k/s; the 4 s window expiry in
                // merge_subnet_window keeps stale swarms from re-arming.
                self.store.reset_subnet_window(sk);
            }
        }
    }
}
