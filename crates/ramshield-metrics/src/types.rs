use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchRecord {
    pub ts_ms: u64,
    pub events: u32,
    pub unique_ips: u32,
    /// Unique IPs promoted to full tracking
    pub promoted: u32,
    /// Unique IPs skipped (below promotion threshold)
    pub cold_skipped: u32,
    /// Connection events in promoted IPs
    pub promoted_events: u32,
    /// Connection events in cold-skipped IPs
    pub cold_skipped_events: u32,
    pub blocks: u32,
    pub hot_subnets: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockRecord {
    pub ts_ms: u64,
    pub ip: String,
    pub reason: String,
    pub module: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleStats {
    pub label: String,
    pub events: u64,
    pub errors: u64,
    pub rate_per_sec: f64,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionState {
    Starting,
    Protected,
    Degraded,
    Failed,
    Stopping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub ts_ms: u64,
    pub uptime_secs: u64,
    pub ips_tracked: usize,
    pub blocked_total: u64,
    pub ram_bytes: usize,
    pub ram_limit_mb: usize,
    pub ram_pct: f64,
    pub cpu_usage: f32,
    pub memory_usage_mb: usize,
    pub total_ram_mb: usize,
    pub ipc_requests: u64,
    pub events_ingested: u64,
    pub events_rejected: u64,
    pub frames_rejected_total: u64,
    pub channel_depth: usize,
    pub events_shed: u64,
    pub batches_total: u64,
    pub promotions: u64,
    pub cold_skipped: u64,
    pub blocks_applied: u64,
    pub pipeline: PipelineFlow,
    pub is_healthy: bool,
    pub health_reason: String,
    /// True if the kernel XDP dataplane is loaded and attached. False when
    /// the daemon is running in degraded mode (in-band enforcement only).
    pub xdp_active: bool,
    /// Last committed WAL LSN (0 = no WAL configured).
    pub wal_lsn: u64,
    /// Pending TTL expirations in the enforcement ring.
    pub pending_expirations: u64,
    /// XDP enforcement failures (map full, attach errors). Surface at the
    /// dashboard level so operators can see kernel-side enforcement trouble
    /// without scraping Prometheus.
    pub xdp_apply_failures: u64,
    /// True when configured active XDP has not been successfully reconciled
    /// within the engine's protection freshness window.
    pub xdp_projection_stale: bool,
    pub xdp_configured: bool,
    pub protection_state: ProtectionState,
}

impl Default for DashboardSnapshot {
    fn default() -> Self {
        Self {
            ts_ms: 0,
            uptime_secs: 0,
            ips_tracked: 0,
            blocked_total: 0,
            ram_bytes: 0,
            ram_limit_mb: 0,
            ram_pct: 0.0,
            cpu_usage: 0.0,
            memory_usage_mb: 0,
            total_ram_mb: 0,
            ipc_requests: 0,
            events_ingested: 0,
            events_rejected: 0,
            frames_rejected_total: 0,
            channel_depth: 0,
            events_shed: 0,
            batches_total: 0,
            promotions: 0,
            cold_skipped: 0,
            blocks_applied: 0,
            pipeline: PipelineFlow {
                ingest: 0,
                queued: 0,
                batched: 0,
                promoted: 0,
                merged: 0,
                blocked: 0,
            },
            is_healthy: true,
            health_reason: "initializing".to_string(),
            xdp_active: false,
            wal_lsn: 0,
            pending_expirations: 0,
            xdp_apply_failures: 0,
            xdp_projection_stale: false,
            xdp_configured: false,
            protection_state: ProtectionState::Starting,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubnetRow {
    pub prefix: String,
    pub events: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineFlow {
    pub ingest: u64,
    pub queued: u64,
    pub batched: u64,
    pub promoted: u64,
    pub merged: u64,
    pub blocked: u64,
}
