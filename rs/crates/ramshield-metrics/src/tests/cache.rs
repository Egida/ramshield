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

/// events_rejected_total conflates three failure classes (auth 401s,
/// connection refusals, channel-full event drops). The breakdown
/// counters are the only way an operator can tell "auth-spammed" from
/// "dropping events" — series absent = blind to the split.
#[test]
fn prometheus_exposes_ipc_rejection_breakdown() {
    let m = Metrics::new();
    m.inc_ipc_event_drops(5);
    m.inc_ipc_auth_rejections(3);
    m.inc_ipc_rejected_connections(2);
    let text = m.render_prometheus();
    assert!(text.contains("ramshield_ipc_event_drops_total 5"));
    assert!(text.contains("# TYPE ramshield_ipc_event_drops_total counter"));
    assert!(text.contains("ramshield_ipc_auth_rejections_total 3"));
    assert!(text.contains("ramshield_ipc_rejected_connections_total 2"));
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
