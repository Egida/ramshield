use crate::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastingConfig {
    pub enabled: bool,
    pub ewma_alpha: f64,
    pub hw_beta: f64,
    pub hw_gamma: f64,
    pub seasonality_period: usize,
    pub anomaly_zscore: f64,
    pub min_entropy: f64,
}

impl Default for ForecastingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ewma_alpha: 0.3,
            hw_beta: 0.1,
            hw_gamma: 0.1,
            seasonality_period: 3_600,
            anomaly_zscore: 2.5,
            min_entropy: 2.0,
        }
    }
}
