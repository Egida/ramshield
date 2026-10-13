use super::*;

impl HoltWinters {
    /// Create a new Holt-Winters forecaster.
    pub fn new(alpha: f64, beta: f64, gamma: f64, period: usize) -> Self {
        let p = period.max(1);
        Self {
            level: 0.0,
            trend: 0.0,
            seasonal: vec![0.0; p],
            period: p,
            alpha,
            beta,
            gamma,
            tick: 0,
        }
    }

    /// Ingest observation `y`, update state, return one-step-ahead forecast.
    pub fn update(&mut self, y: f64) -> f64 {
        if self.tick == 0 {
            self.level = y;
            self.tick += 1;
            return y;
        }
        let s = self.tick % self.period;
        let prev = self.level;
        let seas = self.seasonal[s];
        self.level = self.alpha * (y - seas) + (1.0 - self.alpha) * (prev + self.trend);
        self.trend = self.beta * (self.level - prev) + (1.0 - self.beta) * self.trend;
        self.seasonal[s] = self.gamma * (y - self.level) + (1.0 - self.gamma) * seas;
        self.tick += 1;
        // Forecast for the NEXT tick. The seasonal slot at (tick % period)
        // is the one we just updated, so add `period` to read the future slot.
        // (Without this, the forecast always used the just-updated slot,
        // which biases z-scores by collapsing residuals to near-zero on
        // regular cycles.)
        let ns = self.seasonal[(self.tick + self.period) % self.period];
        (self.level + self.trend + ns).max(0.0)
    }
}

impl EwmAVar {
    /// span: effective window length in ticks. alpha = 2/(span+1).
    pub(crate) fn new(span: usize) -> Self {
        let p = span.max(1);
        let alpha = 2.0 / (p as f64 + 1.0);
        Self {
            ewma: 0.0,
            var_ewma: 0.0,
            count: 0,
            alpha,
        }
    }

    /// Feed a new observation, return the current residual z-score:
    /// z = (observation - ewma) / sqrt(var_ewma).
    /// After warmup (< 2 observations), returns 0.0.
    pub(crate) fn update(&mut self, x: f64) -> f64 {
        self.count += 1;
        if self.count == 1 {
            self.ewma = x;
            return 0.0;
        }
        let diff = x - self.ewma;
        self.ewma += self.alpha * diff;
        self.var_ewma = (1.0 - self.alpha) * (self.var_ewma + self.alpha * diff * diff);
        let sigma = self.var_ewma.sqrt();
        if sigma < 1e-9 {
            0.0
        } else {
            diff.abs() / sigma
        }
    }

    #[cfg(test)]
    pub(crate) fn sigma(&self) -> f64 {
        self.var_ewma.sqrt()
    }
}

impl CusumState {
    /// k: slack (typically 0.5). h: threshold (typically 4.0).
    pub(crate) fn new(k: f64, h: f64) -> Self {
        Self {
            s_upper: 0.0,
            s_lower: 0.0,
            k,
            h,
        }
    }

    /// Feed a z-score (already normalized by sigma). Returns true if alarm.
    pub(crate) fn update(&mut self, z: f64) -> bool {
        self.s_upper = (self.s_upper + z - self.k).max(0.0);
        self.s_lower = (self.s_lower - z - self.k).max(0.0);
        self.s_upper > self.h || self.s_lower > self.h
    }

    pub(crate) fn reset(&mut self) {
        self.s_upper = 0.0;
        self.s_lower = 0.0;
    }
}

/// Shannon entropy in bits for a positive-count distribution.
///
/// Callers must pass `total` equal to the sum of `counts`; zero totals return
/// `0.0` instead of producing NaN. Shared by forecasting and CGNAT analysis.
pub fn shannon_entropy(counts: &[u64], total: u64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / total as f64;
            -p * p.log2()
        })
        .sum()
}
