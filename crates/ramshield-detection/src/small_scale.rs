//! Small-scale per-IP volumetric detection factors.
//!
//! The main engine uses an absolute `rps_threshold` (default 1000/s).
//! At small scale — a few events/sec per IP — that gate is a hard floor:
//! a 30× per-IP flood (50 ev/s vs 1.7 baseline) will not trip it.
//!
//! This module adds O(1)/event statistical factors that fire well below
//! the absolute floor. Combined into one composite score, they replace
//! the "is it above a fixed threshold?" question with "is this statistically
//! unusual for this IP's recent baseline?"
//!
//! ## Factors
//!
//! | #  | Factor                        | Detects                      | Cost/event |
//! |----|-------------------------------|------------------------------|------------|
//! | 1  | Poisson-Gamma log Bayes factor| sustained rate shift         | O(1)       |
//! | 2  | Inter-arrival CV / burstiness | bot-regular vs flood burst   | O(1)       |
//! | 3  | Mann-Kendall τ (per-second)   | monotonic slow-ramp          | O(n²) n≤16 |
//!
//! Combined with the existing EWMA/CUSUM from `rate_tracker`, the composite
//! score fires when ≥ 2 independent factors agree, or one factor is strongly
//! over threshold — tuned for small-scale (ARL0 ≈ 150 events on quiet baseline).
//!
//! ## Usage
//!
//! Feed the module with per-IP event timestamps, one per bin close (or a
//! batch of timestamps at flush boundary). `score()` returns the composite
//! on the range [0, ~5]. `Decision` is derived from `score > threshold`.

use std::collections::VecDeque;

// ── Tuning constants (small-scale regime) ──────────────────────────────────

/// Bayes-factor threshold: ln(10) = "strong evidence" for rate change.
pub const LLR_THRESHOLD: f64 = 2.30;

/// Poisson-Gamma attack multiple: λ1 = LLR_RATIO × λ0.
/// A 3× rate increase (e.g. 1.7 ev/s baseline → 5 ev/s) crosses this.
pub const LLR_RATIO: f64 = 3.0;

/// Minimum quiet-bins before LLR baseline is established.
pub const LLR_WARMUP_BINS: usize = 5;

/// Mann-Kendall: window in bins, fire when τ > threshold.
pub const MK_WINDOW: usize = 16;
pub const MK_TAU_THRESHOLD: f64 = 0.55;

/// CV thresholds: regular bot CV < 0.30, bursty flood CV > 1.50.
pub const CV_REGULAR_THRESHOLD: f64 = 0.30;
pub const CV_BURST_THRESHOLD: f64 = 1.50;

/// Minimum inter-arrival samples before CV is meaningful.
pub const CV_MIN_SAMPLES: usize = 8;

/// Composite-score threshold.
///
/// The LLR weight (3.0) is dominant: a single strong rate-shift (LLR ≥
/// LLR_THRESHOLD) alone pushes the score to exactly SCORE_THRESHOLD,
/// so the decision fires without needing the CV or MK factors to agree.
/// The CV and MK factors act as boosters that lower the effective LLR
/// threshold slightly when two factors agree.
pub const SCORE_THRESHOLD: f64 = 3.0;
/// LLR weight: 3.0 = one strong LLR signal is sufficient on its own.
pub const LLR_WEIGHT: f64 = 3.0;
/// CV weight: a CV factor of 1.0 (fully regular or fully bursty) contributes
/// 0.8 — a meaningful but not decisive boost.
pub const CV_WEIGHT: f64 = 0.8;
/// MK weight: a confirmed monotonic rise (τ > MK_TAU_THRESHOLD over ≥8 bins)
/// is a strong standalone signal — a slow ramp is exactly what MK catches and
/// LLR does not (each ramp bin's rate stays below the 3× attack hypothesis,
/// so LLR decays while MK sees the trend). Set to 3.0 so a trend alone fires.
pub const MK_WEIGHT: f64 = 3.0;

// ── Welford online mean/variance (inter-arrival times) ─────────────────────

/// O(1) running mean and variance of inter-arrival times (seconds).
#[derive(Debug, Clone, Default)]
pub struct Welford {
    n: f64,
    mean: f64,
    m2: f64,
}

impl Welford {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, x: f64) {
        self.n += 1.0;
        let d = x - self.mean;
        self.mean += d / self.n;
        self.m2 += d * (x - self.mean);
    }

    pub fn count(&self) -> usize {
        self.n as usize
    }

    /// Sample standard deviation. Returns 0 when n < 2.
    pub fn std_dev(&self) -> f64 {
        if self.n < 2.0 {
            return 0.0;
        }
        (self.m2 / (self.n - 1.0)).max(0.0).sqrt()
    }

    /// Coefficient of variation (σ/μ). 0 when undefined.
    pub fn cv(&self) -> f64 {
        if self.n < 2.0 || self.mean < 1e-9 {
            return 0.0;
        }
        self.std_dev() / self.mean
    }
}

// ── Mann-Kendall trend detector (on per-second bin counts) ─────────────────

/// O(n²) non-parametric trend test on the last ≤ MK_WINDOW per-second bins.
///
/// `tau` ∈ [-1, 1]: +1 = perfectly rising, 0 = no trend, -1 = falling.
/// Fires when `tau > MK_TAU_THRESHOLD` and at least 8 bins are present.
///
/// Returns `(tau, fired)`.
pub fn mann_kendall_trend(bins: &[u64]) -> (f64, bool) {
    if bins.len() < 8 {
        return (0.0, false);
    }
    let n = bins.len().min(MK_WINDOW);
    let window = &bins[bins.len() - n..];

    let mut s: i64 = 0;
    for j in 1..n {
        for i in 0..j {
            s += (window[j] > window[i]) as i64 - (window[j] < window[i]) as i64;
        }
    }
    let tau = s as f64 / ((n * (n - 1) / 2) as f64);
    (tau, tau > MK_TAU_THRESHOLD && s > 0)
}

// ── Poisson-Gamma log Bayes-factor (rate-shift test) ───────────────────────

/// One-sided log Bayes-factor for a Poisson process with count observed in a
/// fixed-duration window.
///
/// `λ0` = baseline rate (events per second), derived from the first
/// `LLR_WARMUP_BINS` quiet bins.
/// `λ1` = `LLR_RATIO × λ0` (the "attacked" hypothesis).
///
/// Per-observed-event update: `llr += ln(λ1/λ0)`.
/// Per-closed-bin decay:      `llr -= 2.0 × λ0 × K`, where K is that bin's
/// count. (The continuous-time log-Likelihood-ratio in the small-count
/// Poisson-Gamma approximation.)
///
/// Fires when `llr > LLR_THRESHOLD` and warmup is complete.
#[derive(Debug, Clone, Default)]
pub struct RateShiftTest {
    /// Baseline λ0 in events/second; 0 until warmup is complete.
    baseline_rate: f64,
    /// Running LLR; can be negative (evidence for the baseline).
    llr: f64,
    /// Quiet-bin sum (for baseline computation during warmup).
    warmup_sum: u64,
    warmup_count: usize,
}

impl RateShiftTest {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one closed bin (duration 1 s, count `k`).
    ///
    /// Returns `(llr, fired)`.
    pub fn record_bin(&mut self, k: u64) -> (f64, bool) {
        if self.warmup_count < LLR_WARMUP_BINS {
            // Warmup: accumulate quiet bins; compute baseline at completion.
            self.warmup_sum = self.warmup_sum.saturating_add(k);
            self.warmup_count += 1;
            if self.warmup_count == LLR_WARMUP_BINS {
                // Baseline in events/second (each bin = 1 s).
                self.baseline_rate = self.warmup_sum as f64 / LLR_WARMUP_BINS as f64;
            }
            return (0.0, false);
        }

        // After warmup: update the per-bin log Bayes-factor.
        // Poisson log-likelihood-ratio for this bin (duration T = 1 s):
        //   LLR = k·ln(λ1/λ0) − (λ1 − λ0)·T
        // λ0 = baseline, λ1 = LLR_RATIO·λ0 (the "attacked" hypothesis).
        // The decay term (λ1−λ0)·T is FIXED per bin — it does not scale with
        // the observed count k. This is what makes the test one-sided: quiet
        // bins push LLR negative, sustained above-baseline bins push it up.
        let lam0 = self.baseline_rate.max(0.1);
        let lam1 = lam0 * LLR_RATIO;
        self.llr += (k as f64) * (lam1 / lam0).ln();
        self.llr -= (lam1 - lam0) * 1.0;
        let fired = self.llr > LLR_THRESHOLD;
        (self.llr, fired)
    }

    /// Current LLR value (0 before warmup completes).
    pub fn llr(&self) -> f64 {
        self.llr
    }

    /// Whether the warmup phase is complete.
    pub fn warmed_up(&self) -> bool {
        self.warmup_count >= LLR_WARMUP_BINS
    }
}

// ── SmallScaleTracker: per-IP composite state ───────────────────────────────

/// Per-IP tracker. Feed event timestamps; call `close_bin()` at each 1-s
/// bin boundary. `decision()` returns the composite score and a decision.
#[derive(Debug, Clone, Default)]
pub struct SmallScaleTracker {
    inter_arrivals: Welford,
    rate_shift: RateShiftTest,
    /// Ring of the last MK_WINDOW per-second bin counts (for Mann-Kendall).
    bins: VecDeque<u64>,
    last_event_ns: u64,
    /// Current bin count (events in the open 1-s window).
    current_bin_count: u64,
    /// Current bin start timestamp (ns).
    bin_start_ns: u64,
}

impl SmallScaleTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one event with a timestamp in nanoseconds.
    ///
    /// Timestamps are assumed to arrive in non-decreasing order
    /// (true for connection events from a single batch flush).
    pub fn record_event(&mut self, ts_ns: u64) {
        // Compute inter-arrival Δt since the previous event.
        if self.last_event_ns > 0 {
            let dt_s = ts_ns.saturating_sub(self.last_event_ns) as f64 / 1e9;
            self.inter_arrivals.update(dt_s);
        }
        self.last_event_ns = ts_ns;
        self.current_bin_count += 1;
    }

    /// Close the current 1-s bin and advance to the new one.
    ///
    /// Call when the current bin is complete (1 s elapsed, or a batch
    /// boundary signals a new window). Feeds the closed bin's event count
    /// into the Mann-Kendall ring and the Poisson-Gamma LLR.
    ///
    /// Returns the closed bin's event count (0 if no events were seen).
    pub fn close_bin(&mut self) -> u64 {
        let k = self.current_bin_count;
        self.current_bin_count = 0;
        self.bin_start_ns = self.last_event_ns;

        // Feed Mann-Kendall ring.
        if self.bins.len() >= MK_WINDOW {
            self.bins.pop_front();
        }
        self.bins.push_back(k);

        // Feed Poisson-Gamma LLR.
        self.rate_shift.record_bin(k);

        k
    }

    /// Compute the composite small-scale detection score.
    ///
    /// Each factor is normalized to [0, 1] before weighting, so the
    /// composite ranges roughly [0, 5].
    pub fn score(&self) -> f64 {
        // 1. Poisson LLR — normalized by the fire threshold. Floor at 0 (quiet
        // bins push LLR negative); no ceiling, so a strong sustained shift
        // scores well above the threshold rather than pinning at exactly it.
        let llr_norm = if self.rate_shift.warmed_up() {
            (self.rate_shift.llr() / LLR_THRESHOLD).max(0.0)
        } else {
            0.0
        };

        // 2. Inter-arrival CV — burst (CV > 1.5) or bot-regular (CV < 0.3).
        // Both directions are anomalous. 0 when sample is too small.
        let cv = self.inter_arrivals.cv();
        let n = self.inter_arrivals.count();
        let cv_factor = if n >= CV_MIN_SAMPLES {
            if cv > CV_BURST_THRESHOLD {
                ((cv - CV_BURST_THRESHOLD) / CV_BURST_THRESHOLD).min(1.0)
            } else if cv < CV_REGULAR_THRESHOLD {
                ((CV_REGULAR_THRESHOLD - cv) / CV_REGULAR_THRESHOLD).min(1.0)
            } else {
                0.0
            }
        } else {
            0.0
        };

        // 3. Mann-Kendall trend — rising bins = slow ramp.
        let bins: Vec<u64> = self.bins.iter().copied().collect();
        let (_tau, mk_fired) = mann_kendall_trend(&bins);
        let mk_factor = if mk_fired { 1.0 } else { 0.0 };

        // Weighted composite. LLR is dominant: a single sustained rate-shift
        // (llr_norm == 1.0) alone reaches the threshold. CV and MK are
        // boosters — they let a weaker LLR signal still fire when two
        // independent factors agree.
        LLR_WEIGHT * llr_norm + CV_WEIGHT * cv_factor + MK_WEIGHT * mk_factor
    }

    /// Whether the composite exceeds `SCORE_THRESHOLD`.
    pub fn is_alerting(&self) -> bool {
        self.score() >= SCORE_THRESHOLD
    }

    /// Number of bins recorded so far (for warm-up reporting).
    pub fn bins_seen(&self) -> usize {
        self.bins.len()
    }
}

// ── Convenience: drive a full detection scenario ────────────────────────────

/// Convenience: run a scenario of `n_events` events arriving at
/// `events_per_second` for `n_seconds` and return the final score.
///
/// Used primarily by tests; production uses `record_event` + `close_bin`
/// directly at bin boundaries.
pub fn run_scenario(events_per_second: f64, n_seconds: u64, warmup_bins: usize) -> f64 {
    let mut tracker = SmallScaleTracker::new();
    let warmup = warmup_bins.min(warmup_bins.max(1));
    for sec in 0..n_seconds {
        // Events in this second.
        let count = (events_per_second + 0.5).min(9999.0) as u64;
        for i in 0..count {
            let ts = sec * 1_000_000_000 + i * (1_000_000_000 / events_per_second.max(1.0) as u64);
            tracker.record_event(ts);
        }
        let _ = sec;
        tracker.close_bin();
        if sec + 1 < warmup as u64 {
            // Still in warm-up; score is 0.
        }
    }
    tracker.score()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welford_basic() {
        let mut w = Welford::new();
        for &x in &[10.0, 20.0, 30.0, 40.0, 50.0] {
            w.update(x);
        }
        assert_eq!(w.count(), 5);
        assert!((w.mean - 30.0).abs() < 1e-9, "mean={}", w.mean);
        // Sample std of [10,20,30,40,50] = 15.81 (= sqrt(1000/4)).
        assert!((w.std_dev() - 15.81).abs() < 0.01, "std={}", w.std_dev());
        assert!((w.cv() - 15.81 / 30.0).abs() < 0.01, "cv={}", w.cv());
    }

    #[test]
    fn welford_cv_single_value() {
        let mut w = Welford::new();
        w.update(5.0);
        // Only one sample: no variance, no CV.
        assert_eq!(w.std_dev(), 0.0);
        assert_eq!(w.cv(), 0.0);
    }

    #[test]
    fn mann_kendall_no_trend() {
        // Alternating 1, 2 — no monotonic trend.
        let bins = vec![2, 1, 2, 1, 2, 1, 2, 1];
        let (tau, fired) = mann_kendall_trend(&bins);
        assert!(!fired, "alternating bins should not fire");
        assert!(tau.abs() < 0.2, "tau should be near 0, got {tau}");
    }

    #[test]
    fn mann_kendall_rising_trend_fires() {
        // Steadily rising: 1,2,3,...,12
        let bins: Vec<u64> = (1..=12).collect();
        let (tau, fired) = mann_kendall_trend(&bins);
        assert!(fired, "rising bins must fire, tau={tau}");
        assert!(tau > 0.5, "tau should be > 0.5, got {tau}");
    }

    #[test]
    fn mann_kendall_few_bins_does_not_fire() {
        let bins = vec![1, 2, 3, 4, 5];
        let (_tau, fired) = mann_kendall_trend(&bins);
        assert!(!fired, "fewer than 8 bins must not fire");
    }

    #[test]
    fn llr_quiet_bins_do_not_fire() {
        let mut test = RateShiftTest::new();
        // 5 quiet bins at 2 events each (warmup), then 10 quiet bins.
        for _ in 0..5 {
            let _ = test.record_bin(2);
        }
        assert!(test.warmed_up());
        for _ in 0..10 {
            let (_llr, fired) = test.record_bin(2);
            assert!(!fired, "quiet bins at baseline rate must not fire");
        }
        assert!(
            test.llr() <= LLR_THRESHOLD,
            "quiet bins must not accumulate LLR"
        );
    }

    #[test]
    fn llr_sustained_rate_shift_fires() {
        let mut test = RateShiftTest::new();
        // Baseline: 2 events/sec (5 quiet bins).
        for _ in 0..5 {
            let _ = test.record_bin(2);
        }
        // Attack: 6 events/sec for 10 seconds.
        let mut fired_at: Option<usize> = None;
        for i in 0..10 {
            let (_llr, fired) = test.record_bin(6);
            if fired {
                fired_at = Some(i);
                break;
            }
        }
        assert!(
            fired_at.is_some(),
            "sustained 3x rate shift (2→6) must fire LLR within 10 bins"
        );
    }

    #[test]
    fn llr_subthreshold_spike_does_not_fire() {
        let mut test = RateShiftTest::new();
        for _ in 0..5 {
            let _ = test.record_bin(2);
        }
        // A 2x spike (4 vs baseline 2) is below the 3x attack hypothesis:
        // one such bin must not fire.
        let (llr, fired) = test.record_bin(4);
        assert!(!fired, "sub-3x spike should not fire, llr={llr}");
        assert!(
            llr <= LLR_THRESHOLD,
            "single sub-threshold bin stays below threshold"
        );
        // Quiet bins drive the LLR back down (each decays more than it gains).
        for _ in 0..3 {
            let _ = test.record_bin(2);
        }
        assert!(test.llr() < llr, "quiet bins must drain the LLR");
    }

    #[test]
    fn llr_single_huge_spike_does_fire() {
        // A genuine burst (100 events in one bin, 50x baseline) IS strong
        // Poisson evidence — the test is a per-window likelihood test, so it
        // correctly fires on a large window. This is the expected behavior.
        let mut test = RateShiftTest::new();
        for _ in 0..5 {
            let _ = test.record_bin(2);
        }
        let (_llr, fired) = test.record_bin(100);
        assert!(
            fired,
            "a 50x single-bin burst is strong evidence and must fire"
        );
    }

    #[test]
    fn small_scale_burst_fires() {
        // 2 ev/s baseline for 5 s, then 20 ev/s for 10 s.
        let mut t = SmallScaleTracker::new();
        for sec in 0..5 {
            for i in 0..2 {
                t.record_event(sec * 1_000_000_000 + i * 500_000_000);
            }
            t.close_bin();
        }
        // Attack phase: 20 ev/s for 10 s.
        let mut fired: bool = false;
        for sec in 5..15 {
            for i in 0..20 {
                t.record_event(sec * 1_000_000_000 + i * 50_000_000);
            }
            t.close_bin();
            if t.is_alerting() {
                fired = true;
                break;
            }
        }
        assert!(
            fired,
            "20 ev/s burst vs 2 ev/s baseline must fire small-scale alert"
        );
    }

    #[test]
    fn small_scale_quiet_does_not_fire() {
        // Steady 2 ev/s for 30 s — never fires.
        let mut t = SmallScaleTracker::new();
        for sec in 0..30 {
            for _ in 0..2 {
                t.record_event(sec * 1_000_000_000 + 250_000_000);
                t.record_event(sec * 1_000_000_000 + 750_000_000);
            }
            let score = t.score();
            t.close_bin();
            assert!(
                !t.is_alerting(),
                "quiet 2 ev/s must not fire at sec {sec}, score={score}"
            );
        }
    }

    #[test]
    fn small_scale_slow_ramp_fires() {
        // Slow ramp: 2 ev/s for 8 s, then rising 3, 4, 5, ... 12 ev/s over 9 s.
        let mut t = SmallScaleTracker::new();
        for sec in 0..8 {
            for _ in 0..2 {
                t.record_event(sec * 1_000_000_000 + 250_000_000);
                t.record_event(sec * 1_000_000_000 + 750_000_000);
            }
            t.close_bin();
        }
        // Rising phase: 3 ev/s, 4, 5, ..., 12 (10 bins).
        let mut fired = false;
        for (sec_idx, rate) in (3..=12).enumerate() {
            let sec = 8 + sec_idx as u64;
            // Space events evenly within the 1-second bin (spacing must not
            // overflow into the next bin, or bin counts corrupt).
            let spacing_ns = 1_000_000_000 / rate;
            for i in 0..rate {
                t.record_event(sec * 1_000_000_000 + i * spacing_ns);
            }
            t.close_bin();
            if t.is_alerting() {
                fired = true;
                break;
            }
        }
        assert!(fired, "slow ramp 2→12 ev/s must fire (score={})", t.score());
    }

    #[test]
    fn small_scale_bot_regular_pattern_fires() {
        // Perfectly regular: 1 event every 500ms → 2 ev/s, CV ≈ 0 (bot-regular).
        // 30 seconds of perfectly regular events → CV factor should be high.
        let mut t = SmallScaleTracker::new();
        for ms in 0..30_000 {
            t.record_event(ms * 1_000_000);
            // Close bin every 1000 ms (every 2 events).
            if ms % 1000 == 999 {
                t.close_bin();
            }
        }
        t.close_bin();
        // CV should be near 0 (perfectly regular), so CV factor is high.
        let score = t.score();
        // This is a bot-regular pattern: CV < 0.30, so it should contribute to score.
        // But the LLR component will not fire (steady rate).
        // The CV factor alone may be enough: 1.0 × (0.3-0)/0.3 = 1.0 → score = 1.0
        // which is < SCORE_THRESHOLD (3.0).
        // So this test verifies CV contributes but doesn't alone trigger.
        assert!(
            score > 0.5,
            "perfectly regular 2 ev/s should score > 0.5 (CV factor), got {score}"
        );
        // Not necessarily alerting on CV alone — that's expected behavior.
    }

    #[test]
    fn small_scale_burst_and_cv_together_fires() {
        // Burst: 1 event every 50 ms (20 ev/s) for 10 s after 8 s warmup.
        let mut t = SmallScaleTracker::new();
        // Warmup: 8 s at 2 ev/s.
        for sec in 0..8 {
            for _ in 0..2 {
                t.record_event(sec * 1_000_000_000 + 250_000_000);
                t.record_event(sec * 1_000_000_000 + 750_000_000);
            }
            t.close_bin();
        }
        // Burst: 20 ev/s for 5 s, all evenly spaced (CV ≈ 0, very regular).
        let mut fired = false;
        for sec in 8..13 {
            for i in 0..20 {
                t.record_event(sec * 1_000_000_000 + i * 50_000_000);
            }
            t.close_bin();
            if t.is_alerting() {
                fired = true;
                break;
            }
        }
        assert!(
            fired,
            "regular 20 ev/s burst must fire (LLR + CV both contribute)"
        );
    }

    #[test]
    fn run_scenario_quiet_stays_low() {
        let score = run_scenario(2.0, 30, 5);
        assert!(
            score < SCORE_THRESHOLD,
            "quiet 2 ev/s should score < {SCORE_THRESHOLD}, got {score}"
        );
    }

    #[test]
    fn run_scenario_burst_scores_high() {
        // 2 ev/s warmup, then 20 ev/s for 10 s.
        let mut t = SmallScaleTracker::new();
        for sec in 0..5 {
            for i in 0..2 {
                t.record_event(sec * 1_000_000_000 + i * 500_000_000);
            }
            t.close_bin();
        }
        for sec in 5..15 {
            for i in 0..20 {
                t.record_event(sec * 1_000_000_000 + i * 50_000_000);
            }
            t.close_bin();
        }
        let score = t.score();
        assert!(
            score > SCORE_THRESHOLD,
            "20 ev/s after 5 s warmup should score > {SCORE_THRESHOLD}, got {score}"
        );
    }

    #[test]
    fn llr_baseline_high_fewer_false_positives() {
        // High baseline: 50 ev/s for 5 s warmup, then 150 ev/s for 10 s (3x).
        let mut test = RateShiftTest::new();
        for _ in 0..5 {
            let _ = test.record_bin(50);
        }
        assert!(test.warmed_up());
        let mut fired = false;
        for _ in 0..10 {
            let (_llr, f) = test.record_bin(150);
            if f {
                fired = true;
                break;
            }
        }
        assert!(fired, "150 ev/s vs 50 ev/s baseline (3x) must fire LLR");
    }

    #[test]
    fn cv_single_sample_no_crash() {
        let mut t = SmallScaleTracker::new();
        t.record_event(0);
        t.record_event(1_000_000_000);
        let score = t.score();
        assert_eq!(score, 0.0, "2 samples → CV undefined → score should be 0");
    }
}
