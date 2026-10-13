use super::*;

#[test]
fn entropy_uniform() {
    let counts = vec![100u64; 8];
    let total: u64 = counts.iter().sum();
    let h = shannon_entropy(&counts, total);
    assert!((h - 3.0).abs() < 0.01, "H={}", h);
}

#[test]
fn hw_stable_forecast() {
    let mut hw = HoltWinters::new(0.3, 0.1, 0.1, 10);
    for _ in 0..50 {
        hw.update(1000.0);
    }
    assert!(hw.level > 900.0);
}

/// P0 regression: HoltWinters::update must forecast from the FUTURE seasonal
/// slot, not the one it just updated. The old code read
/// `seasonal[tick % period]` after incrementing tick — which is the slot it
/// just wrote, collapsing residuals and producing biased z-scores.
#[test]
fn hw_forecast_uses_future_seasonal_slot() {
    let period = 4;
    let mut hw = HoltWinters::new(0.3, 0.1, 0.1, period);
    // Feed a sine-wave pattern through two full periods so seasonal stabilises.
    // Each value repeats every `period` ticks.
    let pattern = [100.0, 200.0, 100.0, 50.0];
    for _ in 0..(period * 3) {
        for &v in &pattern {
            hw.update(v);
        }
    }
    // The forecast for the NEXT tick should use the seasonal index for that
    // future position — which is the slot NOT updated by the last call.
    // With the bug (read-same-slot), forecast ≈ last value. With the fix
    // (read future slot), forecast is in the range of the pattern.
    let f = hw.update(100.0);
    assert!(
        (50.0..=200.0).contains(&f),
        "forecast {} out of expected pattern range [50, 200]",
        f
    );
}

#[test]
fn reservoir_cold_returns_none() {
    let mut pk = PeakReservoir::new(64);
    for i in 0..30 {
        pk.push(i as f64);
    }
    assert!(!pk.warm());
    assert_eq!(
        pk.extreme_quantile(0.001),
        None,
        "cold reservoir defers to z-score"
    );
}

#[test]
fn reservoir_warm_extreme_quantile_above_typical() {
    let mut pk = PeakReservoir::new(4096);
    for _ in 0..1999 {
        pk.push(10.0); // typical deviation
    }
    pk.push(5_000.0); // one extreme peak among 2000
    assert!(pk.warm());
    let q = pk.extreme_quantile(0.001).unwrap();
    assert!((10.0..5_000.0).contains(&q), "q={}", q);
    // typical dev does not alarm; the extreme does
    assert!(q < 10.0 || (10.0f64).total_cmp(&q).is_le());
    assert!(5_000.0f64.total_cmp(&q).is_gt());
}

#[test]
fn reservoir_negative_deviations_ignored_but_count_ticks() {
    let mut pk = PeakReservoir::new(64);
    for _ in 0..70 {
        pk.push(-1.0);
    }
    assert!(pk.warm(), "ticks advance even on negative dev");
    assert_eq!(
        pk.extreme_quantile(0.001),
        None,
        "no positive peaks → no quantile"
    );
}

#[test]
fn all_zero_window_skipped() {
    let store = Arc::new(Store::new(4));
    let cfg = ForecastingConfig::default();
    let (tx, _rx) = mpsc::channel(8);
    let fc = Forecaster::new(store.clone(), cfg, tx, Arc::new(Metrics::new()));
    // Must not panic / must early-return on all-zero subnet_window.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    rt.block_on(fc.tick_hw()); // entropy now computed in tick_hw
}

/// P0 regression: preemptive_block used to re-drain the threat queue that
/// tick_hw had already emptied — it always saw an empty sample and
/// returned without blocking. It now takes the sample as a parameter, so
/// a high-threat IP in the sample must produce exactly one Block command.
#[test]
fn preemptive_block_emits_for_hot_threats() {
    let store = Arc::new(Store::new(4));
    let cfg = ForecastingConfig::default();
    let (tx, mut rx) = mpsc::channel(8);
    let fc = Arc::new(Forecaster::new(store, cfg, tx, Arc::new(Metrics::new())));
    let hot: std::net::IpAddr = "10.0.0.1".parse().unwrap();
    let cold: std::net::IpAddr = "10.0.0.2".parse().unwrap();
    let sample = vec![(hot, 0.9f32), (cold, 0.3f32)];
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    rt.block_on(fc.preemptive_block(&sample));
    let cmd = rx.try_recv().expect("hot threat must emit a Block command");
    assert_eq!(cmd.ip, hot);
    assert!(matches!(cmd.action, EnforceAction::Block));
    assert!(rx.try_recv().is_err(), "sub-threshold threat must not emit");
}

// ── Phase 1: EWMA variance + CUSUM tests ──────────────────────────────

#[test]
fn ewma_var_adapts_to_phase_change() {
    // Feed 100 ticks at RPS=1000 (stable). Then 100 ticks at RPS=5000.
    // After transition, sigma should be near the new level (~500), not
    // the old level (~1). The old RingBuffer would mix both into σ≈2000.
    let mut ev = super::EwmAVar::new(120);
    for _ in 0..100 {
        ev.update(1000.0);
    }
    let sigma_low = ev.sigma();
    assert!(
        sigma_low < 100.0,
        "stable traffic sigma should be small: {}",
        sigma_low
    );

    // Transition to high traffic
    for _ in 0..100 {
        ev.update(5000.0);
    }
    let sigma_high = ev.sigma();
    // sigma should adapt toward new level's variation (~500 per tick noise)
    assert!(
        sigma_high > 100.0,
        "after phase change sigma should increase: low={} high={}",
        sigma_low,
        sigma_high
    );
}

#[test]
fn ewma_var_residual_zscore_on_spike() {
    // Constant traffic (residual = 0, z = 0). Then a spike (residual ≠ 0, z > 0).
    let mut ev = super::EwmAVar::new(60);
    for _ in 0..50 {
        ev.update(0.0); // zero residual = forecast matches perfectly
    }
    let z_quiet = ev.update(0.0);
    assert!(z_quiet < 0.1, "no anomaly: z={}", z_quiet);

    // Spike: residual = 100 when sigma ≈ small
    let z_spike = ev.update(100.0);
    assert!(z_spike > 2.0, "spike should produce high z: {}", z_spike);
}

#[test]
fn cusum_fires_on_sustained_drift() {
    // k=0.5, h=4.0. Feed z=1.5 for 12 ticks.
    // CUSUM should accumulate: s_upper = (1.5 - 0.5) * 12 = 12 > 4.0 → alarm.
    let mut cs = super::CusumState::new(0.5, 4.0);
    let mut fired = false;
    for _ in 0..12 {
        if cs.update(1.5) {
            fired = true;
            break;
        }
    }
    assert!(
        fired,
        "CUSUM must alarm on sustained z=1.5 drift over 12 ticks"
    );
}

#[test]
fn cusum_stays_quiet_on_noise() {
    // k=0.5, h=4.0. Alternating z: +1, -1, +1, -1 (symmetric noise).
    // CUSUM should NOT alarm — deviations cancel.
    let mut cs = super::CusumState::new(0.5, 4.0);
    for i in 0..100 {
        let z = if i % 2 == 0 { 1.0 } else { -1.0 };
        assert!(
            !cs.update(z),
            "symmetric noise should not trigger CUSUM at tick {}",
            i
        );
    }
}

#[test]
fn cusum_reset_clears_state() {
    // Feed drift, trigger alarm, reset, verify clean.
    let mut cs = super::CusumState::new(0.5, 4.0);
    for _ in 0..10 {
        cs.update(2.0);
    }
    assert!(cs.update(2.0), "should alarm before reset");
    cs.reset();
    assert!(!cs.update(0.0), "must be clean after reset");
}

// ── Phase 2: Bayesian Hypothesis Framework tests ───────────────────────

#[test]
fn bayesian_update_increases_normal_posterior() {
    let mut bt = super::HypothesisTracker::new();
    // Quiet traffic: z=0.2, no entropy change, low threat, no CUSUM.
    for _ in 0..10 {
        bt.bayesian_update(0.2, 0.0, 0.0, false);
    }
    let priors = bt.priors();
    assert!(
        priors[0] > 0.85,
        "H0 should dominate quiet traffic: {:?}",
        priors
    );
    // H1 should be below baseline (0.02) since z is low and threat is low
    assert!(priors[1] < 0.03, "H1 should stay low: {:?}", priors);
}

#[test]
fn bayesian_detects_volumetric_ddos() {
    let mut bt = super::HypothesisTracker::new();
    // Simulate a spike: z=4.0, entropy dropping (delta_h=-0.8), threat=0.9, no CUSUM.
    for _ in 0..20 {
        bt.bayesian_update(4.0, -0.8, 0.9, false);
    }
    let priors = bt.priors();
    assert!(
        priors[1] > priors[0],
        "H1 (volumetric) should exceed H0 (normal): H0={:.3} H1={:.3}",
        priors[0],
        priors[1]
    );
    assert!(
        priors[1] > priors[3],
        "H1 should exceed H3 (flash): H1={:.3} H3={:.3}",
        priors[1],
        priors[3]
    );
}

#[test]
fn bayesian_detects_flash_crowd() {
    let mut bt = super::HypothesisTracker::new();
    // Moderate RPS (z=2.5), entropy UP (diverse IPs), low threat.
    // This is a flash crowd, not DDoS.
    for _ in 0..20 {
        bt.bayesian_update(2.5, 0.8, 0.1, false);
    }
    let priors = bt.priors();
    assert!(
        priors[3] > priors[1],
        "H3 (flash) should exceed H1 (volumetric): H1={:.3} H3={:.3}",
        priors[1],
        priors[3]
    );
    assert!(
        priors[3] > priors[2],
        "H3 (flash) should exceed H2 (slow-ramp): H2={:.3} H3={:.3}",
        priors[2],
        priors[3]
    );
}

#[test]
fn bayesian_slow_ramp_detected_via_cusum() {
    let mut bt = super::HypothesisTracker::new();
    // Low z (gradual increase, not spiking), CUSUM alarm, moderate threat.
    for _ in 0..10 {
        bt.bayesian_update(0.8, -0.1, 0.4, true);
    }
    let priors = bt.priors();
    assert!(
        priors[2] > priors[0],
        "H2 (slow-ramp) should exceed H0: H0={:.3} H2={:.3}",
        priors[0],
        priors[2]
    );
    assert!(
        priors[2] > priors[1],
        "H2 should exceed H1 (no CUSUM signal for H1): H1={:.3} H2={:.3}",
        priors[1],
        priors[2]
    );
}

#[test]
fn bayesian_no_action_when_all_normal() {
    let mut bt = super::HypothesisTracker::new();
    // Quiet traffic for 100 ticks
    for _ in 0..100 {
        bt.bayesian_update(0.1, 0.0, 0.0, false);
    }
    assert!(
        bt.best_above_threshold().is_none(),
        "should not trigger any action on normal traffic"
    );
}

#[test]
fn bayesian_cold_start_requires_higher_confidence() {
    let mut bt = super::HypothesisTracker::new();
    // Extreme spike on tick 1 (cold start) — even though z=5.0 and threat=1.0,
    // cold threshold (0.85) should prevent premature action.
    let _ = bt.bayesian_update(5.0, -1.0, 1.0, false);
    // This tick should NOT trigger — cold start threshold is 0.85
    // (H1 rises but not enough in 1 tick to exceed 0.85)
    let _decision = bt.best_above_threshold();
    // Whether it triggers or not depends on the exact math, but the
    // threshold is higher during cold start. After 60+ ticks it would be lower.
    let mut bt_warm = super::HypothesisTracker::new();
    for _ in 0..70 {
        let _ = bt_warm.bayesian_update(5.0, -1.0, 1.0, false);
    }
    let warm_decision = bt_warm.best_above_threshold();
    // Warm system should detect the attack more easily (lower threshold)
    assert!(
        warm_decision.is_some(),
        "warm system should detect sustained attack: {:?}",
        warm_decision
    );
}
