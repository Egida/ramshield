use super::*;

impl Default for HypothesisTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl HypothesisTracker {
    /// Baseline priors: P(normal)=0.90, P(volumetric)=0.02, P(slow_ramp)=0.02,
    /// P(flash_crowd)=0.05.
    pub(crate) const BASELINE: [f64; H_COUNT] = [0.91, 0.02, 0.02, 0.05]; // sums to 1.0
    pub(crate) const DECAY: f64 = 0.98; // 98% old belief, 2% baseline

    /// Create a new tracker with baseline priors. Uses cold start
    /// threshold for the first 30 ticks.
    pub fn new() -> Self {
        Self {
            priors: Self::BASELINE,
            tick: 0,
            threshold: 0.75,
            cold_threshold: 0.85,
            cold_ticks: 60,
        }
    }

    /// Compute log-likelihoods for each hypothesis given current observations,
    /// then update posteriors via Bayes' rule.
    ///
    /// Inputs: z-score from EWMA variance, entropy delta (current - baseline),
    /// aggregate threat score [0,1], CUSUM alarm flag.
    pub fn bayesian_update(
        &mut self,
        z: f64,
        delta_h: f64,
        threat: f64,
        cusum_alarm: bool,
    ) -> [f64; H_COUNT] {
        self.tick += 1;

        let log_l = [
            log_likelihood_h0(z, delta_h, threat, cusum_alarm),
            log_likelihood_h1(z, delta_h, threat, cusum_alarm),
            log_likelihood_h2(z, delta_h, threat, cusum_alarm),
            log_likelihood_h3(z, delta_h, threat, cusum_alarm),
        ];

        // log P(H_i) = log prior + log likelihood
        let mut log_posterior = [0.0f64; H_COUNT];
        for i in 0..H_COUNT {
            log_posterior[i] = self.priors[i].max(1e-300).ln() + log_l[i];
        }

        // numerically stable softmax
        let max_ll = log_posterior
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
        let mut sum_exp = 0.0f64;
        for (lp, p) in log_posterior.iter().zip(&mut self.priors) {
            *p = (*lp - max_ll).exp();
            sum_exp += *p;
        }
        for p in &mut self.priors {
            *p /= sum_exp;
        }

        // decay toward baseline
        for (p, b) in self.priors.iter_mut().zip(Self::BASELINE) {
            *p = *p * Self::DECAY + b * (1.0 - Self::DECAY);
        }

        self.priors
    }

    /// Return (hypothesis, confidence) if any hypothesis exceeds the
    /// decision threshold. During cold start (< cold_ticks), uses
    /// a higher threshold to prevent premature action.
    pub fn best_above_threshold(&self) -> Option<(Hypothesis, f64)> {
        let eff_threshold = if self.tick < self.cold_ticks {
            self.cold_threshold
        } else {
            self.threshold
        };
        let mut best_idx = 0;
        let mut best_val = 0.0;
        #[allow(clippy::needless_range_loop)]
        for i in 0..H_COUNT {
            if self.priors[i] > best_val {
                best_val = self.priors[i];
                best_idx = i;
            }
        }
        if best_idx == H0 || best_val < eff_threshold {
            return None;
        }
        let h = match best_idx {
            H1 => Hypothesis::VolumetricDoS,
            H2 => Hypothesis::SlowRampDoS,
            H3 => Hypothesis::FlashCrowd,
            _ => return None,
        };
        Some((h, best_val))
    }

    pub fn priors(&self) -> &[f64; H_COUNT] {
        &self.priors
    }
}

// ── Likelihood Functions ──────────────────────────────────────────────────────

/// Clamp a log-likelihood to [-clamp, +clamp] to prevent any single signal
/// from dominating the posterior.
fn clamp_ll(v: f64, clamp: f64) -> f64 {
    v.clamp(-clamp, clamp)
}

/// H₀: Normal traffic. Evidence: z low, entropy stable, threat low, no CUSUM.
fn log_likelihood_h0(z: f64, delta_h: f64, threat: f64, cusum_alarm: bool) -> f64 {
    let mut ll = 0.0;

    // z-score evidence
    ll += if z.abs() < 1.0 {
        0.0
    } else if z.abs() < 2.5 {
        -0.5 * (z.abs() - 1.0)
    } else {
        -1.5
    };

    // entropy evidence
    ll += if delta_h.abs() < 0.3 {
        0.0
    } else {
        -0.5 * (delta_h.abs() - 0.3)
    };

    // threat evidence
    ll += if threat < 0.3 { 0.0 } else { -0.3 * threat };

    // CUSUM evidence
    if cusum_alarm {
        ll -= 2.0;
    }

    clamp_ll(ll, CLAMP_LL)
}

/// H₁: Volumetric DDoS. Evidence: z high, entropy DOWN, threat high.
fn log_likelihood_h1(z: f64, delta_h: f64, threat: f64, _cusum: bool) -> f64 {
    let mut ll = 0.0;

    // z-score: strong support when RPS spike
    ll += if z > 3.0 {
        2.0
    } else if z > 2.5 {
        0.5 * (z - 2.5)
    } else if z < 0.0 {
        -1.0
    } else {
        -0.3 * (2.5 - z).max(0.0)
    };

    // entropy: DDoS shows uniform IPs → entropy drops
    ll += if delta_h < -0.5 {
        1.0
    } else if delta_h < 0.0 {
        0.3
    } else if delta_h > 0.5 {
        -1.5
    } else {
        -0.5
    };

    // threat: high threat = strong DDoS signal
    ll += if threat > 0.8 {
        2.0
    } else if threat > 0.5 {
        1.0
    } else if threat < 0.3 {
        -0.5
    } else {
        0.0
    };

    clamp_ll(ll, CLAMP_LL)
}

/// H₂: Slow-ramp DDoS. Evidence: low z (gradual), CUSUM alarm (primary).
fn log_likelihood_h2(z: f64, delta_h: f64, threat: f64, cusum_alarm: bool) -> f64 {
    let mut ll = 0.0;

    // z-score: neutral if low (slow ramp hasn't spiked yet)
    ll += if z > 2.5 { -0.3 } else { 0.0 };

    // entropy: slight support if dropping
    ll += if delta_h < -0.3 { 0.3 } else { 0.0 };

    // threat: moderate support if elevated
    ll += if threat > 0.3 { 0.5 } else { 0.0 };

    // CUSUM: PRIMARY signal for H₂
    if cusum_alarm {
        ll += 2.5;
    }

    clamp_ll(ll, CLAMP_LL)
}

/// H₃: Flash crowd. Evidence: moderate z (high RPS), entropy UP (diverse IPs),
/// low threat.
fn log_likelihood_h3(z: f64, delta_h: f64, threat: f64, _cusum: bool) -> f64 {
    let mut ll = 0.0;

    // z-score: moderate support if RPS elevated
    ll += if z > 3.0 {
        0.3 // too high — less likely flash crowd
    } else if z > 2.0 {
        0.5
    } else if z < 0.0 {
        -1.0
    } else {
        -0.5
    };

    // entropy: PRIMARY signal for H₃ — flash crowds have diverse IPs
    ll += if delta_h > 0.5 {
        1.5
    } else if delta_h > 0.2 {
        0.8
    } else if delta_h < 0.0 {
        -0.8
    } else {
        -0.3
    };

    // threat: low threat = support for flash crowd
    ll += if threat < 0.3 {
        0.5
    } else if threat > 0.5 {
        -1.0
    } else {
        0.0
    };

    clamp_ll(ll, CLAMP_LL)
}
