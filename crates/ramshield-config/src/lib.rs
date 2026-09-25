use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::sync::Arc;

pub type ConfigHandle = Arc<ArcSwap<Config>>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub engine: EngineConfig,
    #[serde(default)]
    pub detection: DetectionConfig,
    #[serde(default)]
    pub xdp: XdpConfig,
    #[serde(default)]
    pub ipc: IpcConfig,
    #[serde(default)]
    pub forecasting: ForecastingConfig,
    #[serde(default)]
    pub wal: WalConfig,
    #[serde(default)]
    pub dashboard: DashboardConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    pub worker_threads: usize,
    pub ram_limit_mb: usize,
    pub shard_count: usize,
}
impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            worker_threads: 0,
            ram_limit_mb: 512,
            shard_count: 256,
        }
    }
}

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
            promote_min_events: default_promote_min(),
            subnet_window_threshold: default_subnet_window_threshold(),
            emergency_burst_threshold: default_emergency_burst_threshold(),
            pre_aggs_max_size: default_pre_aggs_max_size(),
            pre_aggs_flush_interval_ms: default_pre_aggs_flush_interval_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XdpConfig {
    /// Attach the XDP kernel program. When false, enforcement is in-band only.
    #[serde(default)]
    pub enabled: bool,
    /// Interface to attach to (e.g. "eth0", "lo").
    #[serde(default = "default_xdp_iface")]
    pub interface: String,
    /// "skb" (generic, works everywhere) or "drv" (native, production NICs).
    #[serde(default = "default_xdp_mode")]
    pub mode: String,
}
impl Default for XdpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interface: default_xdp_iface(),
            mode: default_xdp_mode(),
        }
    }
}
fn default_xdp_iface() -> String {
    "eth0".into()
}
fn default_xdp_mode() -> String {
    "skb".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcConfig {
    pub tcp_addr: String,
    pub max_connections: usize,
    #[serde(default = "default_max_connection_bytes")]
    pub max_connection_bytes: Option<usize>,
    #[serde(default = "default_read_timeout_ms")]
    pub read_timeout_ms: Option<u64>,
    #[serde(default = "default_write_timeout_ms")]
    pub write_timeout_ms: Option<u64>,
    #[serde(default = "default_connection_idle_timeout_ms")]
    pub connection_idle_timeout_ms: Option<u64>,
    /// Max line length in bytes (default 32MB). Frames exceeding this are dropped
    /// and the connection is closed. Prevents memory exhaustion from malformed clients.
    #[serde(default)]
    pub max_line_length: Option<usize>,
    /// HMAC-SHA256 keys as `key_id:hex_key` pairs. When non-empty, every IPC
    /// frame MUST carry a valid `{"auth":{"key_id","ts_ms","sig"}}` envelope
    /// (see protocol::auth). Empty = open server (loopback dev default).
    #[serde(default)]
    pub auth_keys: Vec<String>,
    /// Role assignment per key_id. Keys not listed default to `Telemetry`.
    /// Valid roles: `Telemetry` (report only), `ReadOnly` (stats/read),
    /// `Operator` (block/unblock), `Admin` (all).
    /// Example: `key_roles = [{ key_id = "k1", role = "Admin" }]`
    #[serde(default)]
    pub key_roles: Vec<KeyRoleConfig>,
    /// When true, refuse to start (and reject frames) even on loopback if
    /// `auth_keys` is empty. Use in CI/staging to force auth coverage.
    #[serde(default)]
    pub require_auth: bool,
}

/// IPC key roles (P2 authorization).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum KeyRole {
    Telemetry,
    ReadOnly,
    Operator,
    Admin,
}

/// Role assignment for an IPC auth key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRoleConfig {
    pub key_id: String,
    pub role: KeyRole,
}

fn default_max_connection_bytes() -> Option<usize> {
    Some(1_048_576)
}
fn default_read_timeout_ms() -> Option<u64> {
    Some(5000)
}
fn default_write_timeout_ms() -> Option<u64> {
    Some(5000)
}
fn default_connection_idle_timeout_ms() -> Option<u64> {
    Some(30_000)
}
impl Default for IpcConfig {
    fn default() -> Self {
        Self {
            tcp_addr: "127.0.0.1:7890".into(),
            max_connections: 256,
            max_connection_bytes: None,
            read_timeout_ms: None,
            write_timeout_ms: None,
            connection_idle_timeout_ms: None,
            max_line_length: None,
            auth_keys: Vec::new(),
            key_roles: Vec::new(),
            require_auth: false,
        }
    }
}

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

/// WAL durability settings. Disabled by default — enable for crash-durable
/// block state (survives restarts, replays into store + XDP reconcile).
// ponytail: per-field serde(default) omitted — all six keys required in [wal].
// Add `#[serde(default)]` per field (or `#[serde(default)]` on the struct via
// Default impl) when partial [wal] sections should be accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalConfig {
    pub enabled: bool,
    pub dir: String,
    pub durability: ramshield_types::Durability,
    pub compress: bool,
    /// Segment rotation threshold in bytes.
    pub seg_max_bytes: u64,
    /// Max total WAL size on disk. Oldest segments deleted first. 0 = unlimited.
    pub retention_max_bytes: u64,
}
impl Default for WalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: "/var/lib/ramshield/wal".into(),
            durability: ramshield_types::Durability::Flush,
            compress: true,
            seg_max_bytes: 64 * 1024 * 1024,
            retention_max_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardConfig {
    pub enabled: bool,
    /// Bind address for the dashboard HTTP server. Default: `127.0.0.1:9999`.
    /// Override in config.toml to expose on a different port or interface.
    #[serde(default = "default_dashboard_http_addr")]
    pub http_addr: String,
    /// Block-history ring size served by `/api/history/blocks`.
    /// 40 was too small to be useful during floods — entries scrolled out
    /// in seconds. Default raised to 1000.
    #[serde(default = "default_block_log_size")]
    pub block_log_size: usize,
    /// Argon2 PHC hash of the admin password. When set, every dashboard
    /// route except /healthz and /login requires a valid session cookie.
    /// Generate: `echo -n 'pw' | argon2 "$(head -c16 /dev/urandom | xxd -p)" -id -e`
    #[serde(default)]
    pub admin_password_hash: Option<String>,
    /// Session lifetime in seconds (default 8h).
    #[serde(default = "default_session_ttl_secs")]
    pub session_ttl_secs: u64,
    /// Max failed login attempts before lockout (default 50).
    #[serde(default = "default_max_login_attempts")]
    pub max_login_attempts: u32,
    /// Max accepted password length (default 1024). Argon2 work is bounded so
    /// garbage input can't pin the CPU on a multi-MB blob.
    #[serde(default = "default_max_password_length")]
    pub max_password_length: usize,
    /// Reverse proxies trusted to send `X-Forwarded-For` (default: empty =
    /// use the direct TCP peer as the client IP). Each entry is an IP or
    /// CIDR (e.g. `["10.0.0.15", "10.0.1.0/24"]`). Only a peer listed here
    /// may attribute the lockout key to the forwarded client; all other
    /// peers are keyed by TCP peer address. CWE-307 fix.
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
    /// TLS enabled for the dashboard HTTP server. Default: false.
    /// RamShield has no built-in TLS stack — set true only when an external
    /// TLS-terminating reverse proxy fronts the dashboard. When false and
    /// the dashboard binds a non-loopback address, browsers silently drop
    /// `Secure` session cookies on plain HTTP (RFC 6265bis §5.3), causing
    /// infinite login loops with zero server-side errors. CWE-307-adjacent.
    #[serde(default)]
    pub tls_enabled: bool,
    /// Force or disable the Secure flag on session cookies. Default (None):
    /// derive from the bind address — loopback => Secure, non-loopback
    /// plain-HTTP => omit (browsers drop Secure cookies on non-trustworthy
    /// origins). Set true explicitly when running behind an HTTPS terminator.
    #[serde(default)]
    pub cookie_secure: Option<bool>,
}

/// True when `peer` is in `trusted` (exact IP or CIDR member).
/// No new deps — std IpAddr bit-math only. Invalid entries never match.
pub fn peer_is_trusted_proxy(peer: IpAddr, trusted: &[String]) -> bool {
    trusted.iter().any(|e| cidr_contains(e, peer))
}

/// Extract the leftmost client IP from an X-Forwarded-For header value.
/// Leftmost = original client; proxies append to the right. Returns None
/// when absent or unparseable (caller falls back to peer).
pub fn xff_client(header: Option<&str>) -> Option<IpAddr> {
    header?.split(',').next()?.trim().parse().ok()
}

fn cidr_contains(entry: &str, peer: IpAddr) -> bool {
    let entry = entry.trim();
    if !entry.contains('/') {
        return entry.parse::<IpAddr>().ok() == Some(peer);
    }
    let (net, prefix) = match entry.split_once('/') {
        Some(p) => p,
        None => return false,
    };
    let prefix: u32 = match prefix.trim().parse() {
        Ok(p) => p,
        Err(_) => return false,
    };
    match (net.trim().parse::<IpAddr>(), peer) {
        (Ok(IpAddr::V4(n)), IpAddr::V4(p)) if prefix <= 32 => {
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            u32::from(n) & mask == u32::from(p) & mask
        }
        (Ok(IpAddr::V6(n)), IpAddr::V6(p)) if prefix <= 128 => {
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            u128::from(n) & mask == u128::from(p) & mask
        }
        _ => false,
    }
}
fn default_session_ttl_secs() -> u64 {
    28_800
}
fn default_max_login_attempts() -> u32 {
    50
}
fn default_max_password_length() -> usize {
    1024
}
fn default_dashboard_http_addr() -> String {
    "127.0.0.1:9999".into()
}
fn default_block_log_size() -> usize {
    1_000
}
impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            http_addr: "127.0.0.1:9999".into(),
            block_log_size: default_block_log_size(),
            admin_password_hash: None,
            session_ttl_secs: default_session_ttl_secs(),
            max_login_attempts: default_max_login_attempts(),
            max_password_length: default_max_password_length(),
            trusted_proxies: Vec::new(),
            tls_enabled: false,
            cookie_secure: None,
        }
    }
}

/// Sentinel used by the dashboard's GET /api/config for secret fields.
/// api_set_config rejects any patch containing it (POST-back of a viewed
/// config must never boot an auth-less server). pub so both sides share it.
pub const REDACTED_PLACEHOLDER: &str = "<redacted>";

impl Config {
    pub fn from_toml_file(path: &str) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let cfg: Config = toml::from_str(&text)?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Load config from file then apply environment variable overrides.
    /// Env vars take precedence: RAMSHIELD_ENGINE__RAM_LIMIT_MB=1024
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let mut cfg = Self::from_toml_file(path)?;
        cfg.apply_env_overrides()?;
        // P1 fix: file validation runs before overrides, so the final merged
        // configuration is validated again below. Typed env parsing itself is
        // fail-fast and returns an error before validation can be bypassed.
        // RAMSHIELD_IPC__TCP_ADDR=0.0.0.0:7890 (or dashboard addr) without
        // auth keys/hash therefore silently defeated the fail-closed
        // public-bind guard. Validate the FINAL config or fail startup.
        cfg.validate()?;
        Ok(cfg)
    }

    /// Apply RAMSHIELD_*__FIELD environment overrides on top of any config.
    ///
    /// Invalid typed values are fatal rather than silently ignored. This is a
    /// security/operations invariant: an operator must never receive a
    /// successful startup while a requested mitigation threshold, memory
    /// limit, connection limit, or feature toggle was rejected by parsing.
    /// Secret values are never included in error messages.
    pub fn apply_env_overrides(&mut self) -> anyhow::Result<()> {
        fn parse<T>(name: &str) -> anyhow::Result<Option<T>>
        where
            T: std::str::FromStr,
            T::Err: std::fmt::Display,
        {
            match std::env::var(name) {
                Ok(value) => value
                    .parse::<T>()
                    .map(Some)
                    .map_err(|e| anyhow::anyhow!("invalid value for {name}: {e}")),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(std::env::VarError::NotUnicode(_)) => {
                    anyhow::bail!("environment variable {name} is not valid UTF-8")
                }
            }
        }

        // Engine overrides
        if let Some(v) = parse::<usize>("RAMSHIELD_ENGINE__RAM_LIMIT_MB")? {
            self.engine.ram_limit_mb = v;
        }
        if let Some(v) = parse::<usize>("RAMSHIELD_ENGINE__WORKER_THREADS")? {
            self.engine.worker_threads = v;
        }
        if let Some(v) = parse::<usize>("RAMSHIELD_ENGINE__SHARD_COUNT")? {
            self.engine.shard_count = v
                .checked_next_power_of_two()
                .ok_or_else(|| anyhow::anyhow!("RAMSHIELD_ENGINE__SHARD_COUNT is too large"))?;
        }

        // Detection overrides
        if let Some(v) = parse::<u64>("RAMSHIELD_DETECTION__RPS_THRESHOLD")? {
            self.detection.rps_threshold = v;
        }
        if let Some(v) = parse::<u32>("RAMSHIELD_DETECTION__PROMOTE_MIN_EVENTS")? {
            self.detection.promote_min_events = v;
        }
        if let Some(v) = parse::<u64>("RAMSHIELD_DETECTION__BATCH_WINDOW_MS")? {
            self.detection.batch_window_ms = v;
        }
        if let Some(v) = parse::<u64>("RAMSHIELD_DETECTION__SUBNET_WINDOW_THRESHOLD")? {
            self.detection.subnet_window_threshold = v;
        }
        if let Some(v) = parse::<u64>("RAMSHIELD_DETECTION__BLOCK_TTL_SECS")? {
            self.detection.block_ttl_secs = v;
        }
        if let Some(v) = parse::<u64>("RAMSHIELD_DETECTION__SUBNET_BURST_TTL_SECS")? {
            self.detection.subnet_burst_ttl_secs = v;
        }
        if let Some(v) = parse::<u64>("RAMSHIELD_DETECTION__RATE_WINDOW_SECS")? {
            self.detection.rate_window_secs = v;
        }
        if let Some(v) = parse::<usize>("RAMSHIELD_DETECTION__SUBNET_BATCH_THRESHOLD")? {
            self.detection.subnet_batch_threshold = v;
        }
        if let Some(v) = parse::<u64>("RAMSHIELD_DETECTION__SUBNET_BATCH_MIN_EVENTS")? {
            self.detection.subnet_batch_min_events = v;
        }
        if let Some(v) = parse::<bool>("RAMSHIELD_DETECTION__BATCH_BLOCK_ENABLED")? {
            self.detection.batch_block_enabled = v;
        }

        // IPC overrides
        if let Ok(v) = std::env::var("RAMSHIELD_IPC__AUTH_KEYS") {
            self.ipc.auth_keys = v
                .split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect();
        }
        if let Ok(v) = std::env::var("RAMSHIELD_IPC__TCP_ADDR") {
            self.ipc.tcp_addr = v;
        }
        if let Some(v) = parse::<usize>("RAMSHIELD_IPC__MAX_CONNECTIONS")? {
            self.ipc.max_connections = v;
        }

        // Dashboard overrides
        if let Some(v) = parse::<bool>("RAMSHIELD_DASHBOARD__ENABLED")? {
            self.dashboard.enabled = v;
        }
        if let Ok(v) = std::env::var("RAMSHIELD_DASHBOARD__HTTP_ADDR") {
            self.dashboard.http_addr = v;
        }
        if let Ok(v) = std::env::var("RAMSHIELD_DASHBOARD__ADMIN_PASSWORD") {
            use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
            let salt = SaltString::generate(&mut OsRng);
            self.dashboard.admin_password_hash = Some(
                argon2::Argon2::default()
                    .hash_password(v.as_bytes(), &salt)
                    .map_err(|e| {
                        anyhow::anyhow!("failed to hash RAMSHIELD_DASHBOARD__ADMIN_PASSWORD: {e}")
                    })?
                    .to_string(),
            );
        }
        if let Ok(v) = std::env::var("RAMSHIELD_DASHBOARD__ADMIN_PASSWORD_HASH") {
            let v = v.trim().to_string();
            if !v.is_empty() {
                self.dashboard.admin_password_hash = Some(v);
            }
        }

        // Forecasting overrides
        if let Some(v) = parse::<bool>("RAMSHIELD_FORECASTING__ENABLED")? {
            self.forecasting.enabled = v;
        }

        Ok(())
    }

    /// Validate configuration with sensible bounds and error messages.
    pub fn validate(&self) -> anyhow::Result<()> {
        // Engine config validation
        if self.engine.ram_limit_mb < 64 {
            anyhow::bail!("engine.ram_limit_mb must be at least 64 MB");
        }
        if self.engine.shard_count == 0 || !self.engine.shard_count.is_power_of_two() {
            anyhow::bail!("engine.shard_count must be a power of 2");
        }

        // Detection config validation
        if self.detection.rps_threshold == 0 {
            anyhow::bail!("detection.rps_threshold must be > 0");
        }
        if self.detection.promote_min_events == 0 {
            anyhow::bail!("detection.promote_min_events must be > 0");
        }
        if self.detection.bloom_bits < 100_000 {
            anyhow::bail!(
                "detection.bloom_bits should be at least 100,000 for low false positive rate"
            );
        }
        if self.detection.batch_max_events == 0 || self.detection.batch_max_events > 65536 {
            anyhow::bail!("detection.batch_max_events must be between 1 and 65536");
        }
        if self.detection.batch_window_ms == 0 || self.detection.batch_window_ms > 500 {
            anyhow::bail!("detection.batch_window_ms must be between 1 and 500 ms");
        }
        if self.detection.subnet_window_threshold < 10 {
            anyhow::bail!("detection.subnet_window_threshold should be at least 10");
        }
        if self.detection.pre_aggs_max_size == 0 {
            anyhow::bail!("detection.pre_aggs_max_size must be > 0");
        }

        // IPC config validation
        if self.ipc.max_connections == 0 {
            anyhow::bail!("ipc.max_connections must be > 0");
        }
        if self.ipc.max_connections > 1_000_000 {
            anyhow::bail!("ipc.max_connections should not exceed 1,000,000");
        }

        // Forecasting config validation
        if self.forecasting.enabled {
            if !(0.0..=1.0).contains(&self.forecasting.ewma_alpha) {
                anyhow::bail!("forecasting.ewma_alpha must be in range [0.0, 1.0]");
            }
            if self.forecasting.seasonality_period == 0 {
                anyhow::bail!("forecasting.seasonality_period must be > 0");
            }
            if self.forecasting.anomaly_zscore < 1.0 {
                anyhow::bail!("forecasting.anomaly_zscore should be at least 1.0");
            }
        }

        // Dashboard config validation
        if self.dashboard.http_addr.is_empty() {
            anyhow::bail!("dashboard.http_addr must not be empty");
        }

        // Fail-closed: a public bind without credentials is an open admin surface.
        // Loopback (127.0.0.1 / ::1) stays open for local dev.
        if is_public_bind(&self.dashboard.http_addr) && self.dashboard.admin_password_hash.is_none()
        {
            anyhow::bail!(
                "dashboard.http_addr binds a public interface ({}) but admin_password_hash is unset — set the hash or bind 127.0.0.1",
                self.dashboard.http_addr
            );
        }
        if is_public_bind(&self.ipc.tcp_addr) && self.ipc.auth_keys.is_empty() {
            anyhow::bail!(
                "ipc.tcp_addr binds a public interface ({}) but auth_keys is empty — set HMAC keys or bind 127.0.0.1",
                self.ipc.tcp_addr
            );
        }
        if self.ipc.require_auth && self.ipc.auth_keys.is_empty() {
            anyhow::bail!(
                "ipc.require_auth=true but auth_keys is empty — set HMAC keys or disable require_auth"
            );
        }

        // P1e: validate auth_keys shape (key_id:hex) + PHC hash format,
        // before any IPC server binds with them. No new deps — std hex check.
        for entry in &self.ipc.auth_keys {
            let (id, hex_str) = match entry.split_once(':') {
                Some((k, v)) => (k, v),
                None => ("", entry.as_str()),
            };
            // An entry with no key_id (":hex" or a bare hex blob) passes the hex
            // checks below but parse_ipc_keys rejects it at bind time, which used
            // to leave the listener up with ZERO active keys (silent downgrade to
            // unauthenticated). Fail at validate() instead. Never echo `entry` —
            // for a colon-less entry the whole string IS the secret.
            if id.is_empty() {
                anyhow::bail!(
                    "ipc.auth_keys entries must be `key_id:hex_key` — empty or missing key_id"
                );
            }
            if hex_str.is_empty() {
                anyhow::bail!("ipc.auth_keys entries must be `key_id:hex_key`");
            }
            if hex_str.len() % 2 != 0 {
                anyhow::bail!("ipc.auth_keys[{id}] has odd-length hex key");
            }
            if hex_str.bytes().any(|b| !b.is_ascii_hexdigit()) {
                anyhow::bail!("ipc.auth_keys[{id}] contains non-hex characters");
            }
            if hex_str.len() < 32 {
                anyhow::bail!("ipc.auth_keys[{id}] hex key must be >= 32 chars (16 bytes)");
            }
        }
        if let Some(ref p) = self.dashboard.admin_password_hash
            && argon2::PasswordHash::new(p).is_err()
        {
            anyhow::bail!("dashboard.admin_password_hash is not a valid PHC string");
        }

        Ok(())
    }

    /// Go-live exposure notes (P1-7): the stack has NO TLS. A public bind
    /// sends admin credentials plaintext and Secure cookies are never set.
    /// validate() still permits it (operator may front a TLS reverse proxy);
    /// these warnings make the exposure visible at startup.
    pub fn exposure_warnings(&self) -> Vec<String> {
        let mut w = Vec::new();
        if is_public_bind(&self.dashboard.http_addr) {
            w.push(format!(
                "dashboard.http_addr={} is a public bind and RamShield has no TLS — admin \
                 credentials traverse plaintext and Secure cookies are never set. Bind \
                 127.0.0.1 or front with a TLS reverse proxy.",
                self.dashboard.http_addr
            ));
        }
        if !self.dashboard.tls_enabled && !is_loopback_bind(&self.dashboard.http_addr) {
            w.push(format!(
                "dashboard.http_addr={} binds a non-loopback address over plain HTTP \
                 without TLS. Browsers will silently drop the Secure session cookie \
                 on HTTP for non-loopback origins (RFC 6265bis §5.3), causing \
                 infinite login loops with zero server-side errors. \
                 Set dashboard.tls_enabled=true or front with a TLS reverse proxy.",
                self.dashboard.http_addr
            ));
        }
        if is_public_bind(&self.ipc.tcp_addr) {
            w.push(format!(
                "ipc.tcp_addr={} is a public bind — IPC traffic (incl. HMAC frames) is \
                 plaintext. Bind 127.0.0.1 or front with a TLS reverse proxy.",
                self.ipc.tcp_addr
            ));
        }
        w
    }

    pub fn into_handle(self) -> ConfigHandle {
        Arc::new(ArcSwap::from_pointee(self))
    }
}

/// True when `addr` binds an interface reachable from other hosts.
/// Loopback (127.0.0.1, ::1) and link-local (169.254.x.x, fe80::/10) are
/// host-private; all other L3 addresses — including RFC1918 private
/// ranges — are LAN-reachable and treated as public for fail-closed
/// exposure purposes. Hostnames are treated as private (DNS may resolve
/// anywhere; a wrong answer is a config bug, not a code risk).
fn is_public_bind(addr: &str) -> bool {
    let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);
    let host = host.trim_matches(['[', ']']);
    if host.is_empty() || host == "0.0.0.0" || host == "::" || host == "*" {
        return true;
    }
    match host.parse::<IpAddr>() {
        // Loopback and link-local are unreachable from other hosts. Everything
        // else — including RFC1918 private ranges — is reachable from the LAN,
        // so an unauthenticated bind there is an exposure.
        Ok(IpAddr::V4(ip)) => !(ip.is_loopback() || ip.is_link_local()),
        Ok(IpAddr::V6(ip)) => !(ip.is_loopback() || ip.is_unicast_link_local()),
        Err(_) => {
            // ponytail: fail-CLOSED on unparseable hostnames. A hostname like
            // "dashboard.example.com" would otherwise fall through as "private"
            // and skip the public-bind-without-auth bail — an operator could
            // set `dashboard.http_addr = "dashboard.example.com:9999"` and
            // ship a public dashboard with no password. DNS may resolve to a
            // private address (and we can't tell here), but the safe default
            // is to require auth. Upgrade: resolve DNS at startup and cache.
            //
            // EXCEPTION: "localhost" (RFC 6761) is a reserved loopback name.
            // Treat it as non-public so dev workflows without auth still work.
            host != "localhost"
        }
    }
}

/// True when `addr`'s host is a loopback IP (127.0.0.0/8, ::1).
/// Browsers accept `Secure` cookies over plain HTTP only for loopback
/// origins (RFC 6265bis "trustworthy origin"). A non-loopback HTTP bind
/// without TLS makes the browser silently drop the `Secure` session
/// cookie → infinite login loop with no server-side error.
pub fn is_loopback_bind(addr: &str) -> bool {
    let host = addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr);
    let host = host.trim_matches(['[', ']']);
    if host.is_empty() || host == "0.0.0.0" || host == "::" || host == "*" {
        return false;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback(),
        Ok(IpAddr::V6(ip)) => ip.is_loopback(),
        // Hostnames: can't classify statically — treat as non-loopback so
        // the warning fires (a loopback hostname like "localhost" gets a
        // harmless extra warning; a LAN hostname missing the warning is
        // the dangerous direction).
        Err(_) => false,
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// IPv6 plan Task 6: bracketed v6 binds are the documented form
    /// ("[::]:7890"); the public-exposure guard must see through the
    /// brackets or an unauthenticated dashboard binds all-interfaces
    /// while validate() thinks it's loopback-only.
    #[test]
    fn public_bind_covers_v6_forms() {
        assert!(is_public_bind("[::]:7890"), "bracketed any-v6");
        assert!(
            is_public_bind(":::7890"),
            "unbracketed any-v6 host '::' + port"
        );
        assert!(is_public_bind("[*]:7890"), "bracketed star");
        assert!(is_public_bind("[0.0.0.0]:80"), "bracketed v4-any");
        assert!(
            !is_public_bind("[::1]:7890"),
            "bracketed loopback is private"
        );
        assert!(
            !is_public_bind("::1:7890"),
            "unbracketed loopback is private"
        );
        assert!(
            !is_public_bind("[fe80::1]:7890"),
            "bracketed v6 link-local is private"
        );
        assert!(
            !is_public_bind("127.0.0.1:7890"),
            "v4 loopback stays private"
        );
    }

    /// P1: NIC-specific binds reach the LAN. An unauthenticated dashboard on
    /// 192.168.x.x is one ARP hop from every host on the segment — the
    /// fail-closed guard must not treat it as operator-local. Hostnames stay
    /// private (localhost resolves loopback; a DNS name needs an answer we
    /// can't get here). Link-local is unreachable from other hosts.
    #[test]
    fn lan_nic_binds_are_public() {
        assert!(is_public_bind("192.168.1.5:9999"), "v4 private LAN");
        assert!(is_public_bind("10.0.0.1:7890"), "v4 private 10/8");
        assert!(is_public_bind("172.16.0.1:7890"), "v4 private 172.16/12");
        assert!(is_public_bind(":9999"), "empty host binds 0.0.0.0");
        assert!(!is_public_bind("[fe80::1]:7890"), "v6 link-local private");
        // ponytail: 'localhost' (RFC 6761) is the only hostname exempted from
        // fail-closed. All other unparseable hostnames are treated as public
        // to prevent shipping a passwordless dashboard on a DNS name.
        assert!(
            !is_public_bind("localhost:9999"),
            "localhost is RFC 6761 loopback"
        );
        assert!(
            is_public_bind("dashboard.example.com:9999"),
            "arbitrary hostname is fail-closed public"
        );
    }

    /// Vuln 3 (Secure cookie lockout on non-loopback HTTP): the loopback
    /// classifier is the mirror of is_public_bind but stricter — it only
    /// accepts 127.0.0.0/8 and ::1. Browsers accept `Secure` cookies over
    /// plain HTTP only for loopback origins (RFC 6265bis §5.3 "trustworthy
    /// origin"); a non-loopback bind without TLS silently drops the cookie.
    #[test]
    fn loopback_bind_only_accepts_loopback() {
        assert!(is_loopback_bind("127.0.0.1:9999"), "v4 loopback");
        assert!(is_loopback_bind("127.0.0.0:9999"), "v4 loopback net base");
        assert!(
            is_loopback_bind("127.255.255.255:9999"),
            "v4 loopback net top"
        );
        assert!(is_loopback_bind("[::1]:9999"), "v6 loopback bracketed");
        assert!(is_loopback_bind("::1:9999"), "v6 loopback unbracketed");
        assert!(!is_loopback_bind("192.168.1.5:9999"), "LAN is non-loopback");
        assert!(
            !is_loopback_bind("10.0.0.1:7890"),
            "private is non-loopback"
        );
        assert!(!is_loopback_bind("0.0.0.0:9999"), "any-v4 is non-loopback");
        assert!(!is_loopback_bind("[::]:9999"), "any-v6 is non-loopback");
        assert!(!is_loopback_bind(":9999"), "empty host is non-loopback");
        assert!(
            !is_loopback_bind("localhost:9999"),
            "hostname is non-loopback"
        );
    }

    /// Vuln 3: exposure_warnings must fire the Secure-cookie warning for a
    /// non-loopback bind without TLS, and must stay silent for loopback
    /// (the default 127.0.0.1:9999) and for tls_enabled=true.
    #[test]
    fn secure_cookie_warning_fires_off_loopback() {
        let mut c = Config::default();
        // Default: loopback, no TLS → the loopback-Secure warning does not fire.
        let secure_warn = |w: &str| w.contains("Secure session cookie");
        assert!(
            !c.exposure_warnings().iter().any(|w| secure_warn(w)),
            "loopback must not fire the Secure-cookie HTTP warning"
        );
        // LAN bind without TLS → the loopback-Secure warning fires.
        c.dashboard.http_addr = "192.168.1.50:9999".into();
        assert!(
            c.exposure_warnings().iter().any(|w| secure_warn(w)),
            "LAN bind without TLS must fire the Secure-cookie HTTP warning"
        );
        // Same bind with TLS fronting → the loopback-Secure warning silenced.
        c.dashboard.tls_enabled = true;
        assert!(
            !c.exposure_warnings().iter().any(|w| secure_warn(w)),
            "tls_enabled=true must silence the Secure-cookie HTTP warning"
        );
    }

    #[cfg(test)]
    fn clear_env_vars() {
        let keys = [
            "RAMSHIELD_ENGINE__RAM_LIMIT_MB",
            "RAMSHIELD_ENGINE__WORKER_THREADS",
            "RAMSHIELD_ENGINE__SHARD_COUNT",
            "RAMSHIELD_DETECTION__RPS_THRESHOLD",
            "RAMSHIELD_DETECTION__PROMOTE_MIN_EVENTS",
            "RAMSHIELD_DETECTION__BATCH_WINDOW_MS",
            "RAMSHIELD_DETECTION__SUBNET_WINDOW_THRESHOLD",
            "RAMSHIELD_DETECTION__BLOCK_TTL_SECS",
            "RAMSHIELD_IPC__TCP_ADDR",
            "RAMSHIELD_IPC__MAX_CONNECTIONS",
            "RAMSHIELD_DASHBOARD__ENABLED",
            "RAMSHIELD_DASHBOARD__HTTP_ADDR",
            "RAMSHIELD_FORECASTING__ENABLED",
        ];
        for k in &keys {
            unsafe {
                std::env::remove_var(k);
            }
        }
    }

    #[test]
    fn default_config_validates() {
        let cfg = Config::default();
        cfg.validate().unwrap();
        assert!(
            cfg.exposure_warnings().is_empty(),
            "loopback-only config must warn about nothing"
        );
    }

    #[test]
    fn public_binds_produce_exposure_warnings() {
        let mut cfg = Config::default();
        cfg.dashboard.http_addr = "0.0.0.0:9999".into();
        cfg.dashboard.admin_password_hash = Some("$argon2id$v=19$m=19456,t=2,p=1$rOGcgxnibWHynWZ0exEH7Q$KM06+4aIAIc2nPNe+jyGekH+zqzAwwYw3JHzgo26b1M".into());
        cfg.ipc.tcp_addr = "0.0.0.0:7890".into();
        cfg.ipc.auth_keys =
            vec!["k1:0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f2021".into()];
        // validate() still permits (operator may TLS-front); warnings must flag it.
        cfg.validate().unwrap();
        let w = cfg.exposure_warnings();
        assert!(
            w.iter().any(|s| s.contains("dashboard.http_addr")),
            "expected dashboard warning, got {w:?}"
        );
        assert!(
            w.iter().any(|s| s.contains("ipc.tcp_addr")),
            "expected ipc warning, got {w:?}"
        );
    }

    #[test]
    fn public_dashboard_without_password_is_rejected() {
        let mut cfg = Config::default();
        cfg.dashboard.http_addr = "0.0.0.0:9999".into();
        cfg.dashboard.admin_password_hash = None;
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("admin_password_hash"), "{err}");
    }

    #[test]
    fn public_ipc_without_keys_is_rejected() {
        let mut cfg = Config::default();
        cfg.ipc.tcp_addr = "0.0.0.0:7890".into();
        cfg.ipc.auth_keys.clear();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("auth_keys"), "{err}");
    }

    #[test]
    fn loopback_without_auth_still_validates() {
        let cfg = Config::default();
        assert_eq!(cfg.dashboard.http_addr, "127.0.0.1:9999");
        assert_eq!(cfg.ipc.tcp_addr, "127.0.0.1:7890");
        cfg.validate().unwrap();
    }

    #[test]
    fn require_auth_on_loopback_without_keys_is_rejected() {
        let mut cfg = Config::default();
        cfg.ipc.require_auth = true;
        cfg.ipc.auth_keys.clear();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("require_auth"), "{err}");
    }

    #[test]
    fn auth_key_without_key_id_is_rejected() {
        // A bare hex blob (no `:`) or `:hex` passes the length/hex checks below
        // it, but parse_ipc_keys rejects the entry at bind time — which used to
        // leave the listener up with ZERO active keys (silent downgrade to
        // unauthenticated). validate() must fail first.
        let hex = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f2021";
        let mut cfg = Config::default();

        cfg.ipc.auth_keys = vec![hex.into()];
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("key_id"), "{err}");

        cfg.ipc.auth_keys = vec![format!(":{hex}")];
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("key_id"), "{err}");

        // Sanity: the same secret WITH a key_id is accepted.
        cfg.ipc.auth_keys = vec![format!("k1:{hex}")];
        cfg.validate().unwrap();
    }

    #[test]
    fn require_auth_with_valid_key_validates() {
        let mut cfg = Config::default();
        cfg.ipc.require_auth = true;
        cfg.ipc.auth_keys =
            vec!["k1:0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f2021".into()];
        cfg.validate().unwrap();
    }

    #[test]
    fn subnet_burst_ttl_default_is_short_and_serde_defaults_apply() {
        // Regression: subnet batch blocks used to inherit block_ttl_secs (1h),
        // locking out whole /24s of shared egress for an hour.
        let cfg = Config::default();
        assert_eq!(cfg.detection.subnet_burst_ttl_secs, 120);
        assert!(cfg.detection.subnet_burst_ttl_secs < cfg.detection.block_ttl_secs);
        // Old TOML without the field must still parse (serde default) —
        // parse just the [detection] table; other tables have their own requireds.
        let parsed: DetectionConfig = toml::from_str(
            "rps_threshold = 100\nrate_window_secs = 10\nsubnet_batch_threshold = 50\nsubnet_batch_min_events = 100\nbatch_block_enabled = true\nblock_ttl_secs = 3600\nbloom_bits = 1000",
        )
        .unwrap();
        assert_eq!(parsed.subnet_burst_ttl_secs, 120);
    }

    #[test]
    #[serial]
    fn env_var_override_ram_limit() {
        clear_env_vars();
        unsafe {
            std::env::set_var("RAMSHIELD_ENGINE__RAM_LIMIT_MB", "1024");
        }
        let tmpfile = "/tmp/ramshield_test_config.toml";
        std::fs::write(tmpfile, "").unwrap();
        let cfg = Config::load(tmpfile).unwrap();
        assert_eq!(cfg.engine.ram_limit_mb, 1024);
        clear_env_vars();
    }

    #[test]
    #[serial]
    fn env_override_detection_rps() {
        clear_env_vars();
        unsafe {
            std::env::set_var("RAMSHIELD_DETECTION__RPS_THRESHOLD", "500");
        }
        let tmpfile = "/tmp/ramshield_test_config.toml";
        std::fs::write(tmpfile, "").unwrap();
        let cfg = Config::load(tmpfile).unwrap();
        assert_eq!(cfg.detection.rps_threshold, 500);
        clear_env_vars();
    }

    /// P1 regression: env override must not smuggle a public bind past the
    /// fail-closed validation that only ran on the file. Before the fix,
    /// Config::load validated the file, applied env overrides, and discarded
    /// the re-validation result — so this env combo booted an open server.
    #[test]
    #[serial]
    fn env_override_public_bind_is_rejected() {
        clear_env_vars();
        unsafe {
            std::env::set_var("RAMSHIELD_IPC__TCP_ADDR", "0.0.0.0:7890");
        }
        let tmpfile = "/tmp/ramshield_test_config.toml";
        std::fs::write(tmpfile, "").unwrap();
        let err =
            Config::load(tmpfile).expect_err("public IPC bind without auth_keys must fail startup");
        assert!(err.to_string().contains("auth_keys"), "{err}");
        clear_env_vars();
    }

    #[test]
    #[serial]
    fn env_override_invalid_value_is_rejected() {
        clear_env_vars();
        unsafe {
            std::env::set_var("RAMSHIELD_ENGINE__RAM_LIMIT_MB", "not_a_number");
        }
        let tmpfile = "/tmp/ramshield_test_config.toml";
        std::fs::write(tmpfile, "").unwrap();
        let err = Config::load(tmpfile).expect_err("invalid typed env override must fail startup");
        assert!(
            err.to_string().contains("RAMSHIELD_ENGINE__RAM_LIMIT_MB"),
            "{err}"
        );
        clear_env_vars();
    }
}
