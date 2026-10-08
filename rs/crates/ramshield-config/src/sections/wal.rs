use crate::*;

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
    /// When WAL open/replay fails, continue without durability (volatile).
    /// Default false: WAL enabled + open/replay failure = startup failure.
    #[serde(default)]
    pub allow_volatile_fallback: bool,
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
            allow_volatile_fallback: false,
        }
    }
}
