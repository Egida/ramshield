use crate::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionConfig {
    pub rps_threshold: u64,
    pub rate_window_secs: u64,
    /// Unique IPs per /24 in one window required for a subnet batch block.
    /// Keyed on unique IPs (not raw events): one abuser at 500 events is a
    /// single offender; 50 IPs × 12 events is a swarm. Old default of 5 events
    /// blocked whole /24s on a single 10-event burst — CGNAT killer.
    pub subnet_batch_threshold: usize,
    /// /24 event volume in the same window, secondary gate: block requires
    /// BOTH unique_ips >= subnet_batch_threshold AND events >= this.
    #[serde(default = "default_subnet_batch_min_events")]
    pub subnet_batch_min_events: u64,
    pub batch_block_enabled: bool,
    pub block_ttl_secs: u64,
    /// Pulse-wave correlation: sliding window (seconds) to detect short
    /// bursts spaced just below detection threshold. Sized for 2s-on/3s-off
    /// T13 pattern (window covers 1 gap + 1 burst).
    #[serde(default = "default_pulse_window_secs")]
    pub pulse_window_secs: u64,
    /// Distinct over-threshold samples within pulse_window_secs that
    /// escalate to a pulse-wave block. Set to 2; raise if FPR measured.
    #[serde(default = "default_pulse_threshold_samples")]
    pub pulse_threshold_samples: u8,
    /// TTL for subnet_burst blocks specifically. Shared egress /24s hold up to
    /// 253 hosts; inheriting the 1h per-IP TTL locked out whole CGNAT ranges
    /// for an hour. Short default — continued abuse re-fires from fresh events.
    #[serde(default = "default_subnet_burst_ttl_secs")]
    pub subnet_burst_ttl_secs: u64,
    pub bloom_bits: usize,
    /// Max events accumulated before a forced flush (high-traffic batching).
    #[serde(default = "default_batch_max_events")]
    pub batch_max_events: usize,
    /// Max wait (ms) before flushing a partial batch.
    #[serde(default = "default_batch_window_ms")]
    pub batch_window_ms: u64,
    /// Max wait (ms) before flushing the pre-aggregation buffer.
    #[serde(default = "default_pre_aggs_flush_interval_ms")]
    pub pre_aggs_flush_interval_ms: u64,
    /// Per-IP hits required in one window before full IpRecord tracking.
    #[serde(default = "default_promote_min")]
    pub promote_min_events: u32,
    /// /24 event count in one window that lowers promotion threshold for that subnet.
    #[serde(default = "default_subnet_window_threshold")]
    pub subnet_window_threshold: u64,
    /// Per-IP emergency burst gate: when one IP emits this many events in a
    /// single unflushed detection window (~pre_aggs_flush_interval), the
    /// worker emits an in-flight block immediately instead of waiting for
    /// the periodic flush (50-1000ms of uninhibited traffic otherwise).
    /// 0 disables the fast path (flush-only detection).
    #[serde(default = "default_emergency_burst_threshold")]
    pub emergency_burst_threshold: u32,
    /// Max unique IPs in the pre-aggregation buffer before flushing to main store.
    #[serde(default = "default_pre_aggs_max_size")]
    pub pre_aggs_max_size: usize,
    /// Opt-in relative-baseline detector. Default off until qualified in the
    /// target environment. Runs only for promoted IP records.
    #[serde(default)]
    pub relative_enabled: bool,
    /// Relative multiplier over the prior slow baseline. Must be finite and >= 1.
    #[serde(default = "default_relative_factor")]
    pub relative_factor: f64,
    /// Absolute floor for the relative detector. Must be finite and > 0.
    #[serde(default = "default_relative_floor_rps")]
    pub relative_floor_rps: f64,
    /// Number of observed samples required before relative detection can fire.
    /// IpRecord::sample_count is u8, so this is intentionally capped at 255.
    #[serde(default = "default_relative_min_samples")]
    pub relative_min_samples: u32,
    /// Consecutive relative breaches required to fire. Must be > 0.
    #[serde(default = "default_relative_min_breaches")]
    pub relative_min_breaches: u8,
}

fn default_batch_max_events() -> usize {
    4096
}

fn default_batch_window_ms() -> u64 {
    50
}

fn default_promote_min() -> u32 {
    8
}

fn default_subnet_batch_min_events() -> u64 {
    100
}

fn default_pulse_window_secs() -> u64 {
    6
}

fn default_pulse_threshold_samples() -> u8 {
    2
}

fn default_subnet_burst_ttl_secs() -> u64 {
    120
}

fn default_subnet_window_threshold() -> u64 {
    500
}

fn default_pre_aggs_max_size() -> usize {
    1_000_000
}

fn default_pre_aggs_flush_interval_ms() -> u64 {
    1000
}

fn default_emergency_burst_threshold() -> u32 {
    500
}

fn default_relative_factor() -> f64 {
    5.0
}

fn default_relative_floor_rps() -> f64 {
    3.0
}

fn default_relative_min_samples() -> u32 {
    8
}

fn default_relative_min_breaches() -> u8 {
    3
}

impl Default for DetectionConfig {
    fn default() -> Self {
        Self {
            rps_threshold: 1_000,
            rate_window_secs: 10,
            subnet_batch_threshold: 50,
            subnet_batch_min_events: default_subnet_batch_min_events(),
            batch_block_enabled: true,
            block_ttl_secs: 3_600,
            pulse_window_secs: default_pulse_window_secs(),
            pulse_threshold_samples: default_pulse_threshold_samples(),
            subnet_burst_ttl_secs: default_subnet_burst_ttl_secs(),
            bloom_bits: 8_000_000,
            batch_max_events: default_batch_max_events(),
            batch_window_ms: default_batch_window_ms(),
            pre_aggs_flush_interval_ms: default_pre_aggs_flush_interval_ms(),
            promote_min_events: default_promote_min(),
            subnet_window_threshold: default_subnet_window_threshold(),
            emergency_burst_threshold: default_emergency_burst_threshold(),
            pre_aggs_max_size: default_pre_aggs_max_size(),
            relative_enabled: false,
            relative_factor: default_relative_factor(),
            relative_floor_rps: default_relative_floor_rps(),
            relative_min_samples: default_relative_min_samples(),
            relative_min_breaches: default_relative_min_breaches(),
        }
    }
}
