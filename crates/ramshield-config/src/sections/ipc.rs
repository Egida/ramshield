use crate::*;

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
    /// Operator assertion that a TLS/mTLS proxy terminates in front of a
    /// non-loopback IPC bind. HMAC does not encrypt transport; public bind
    /// without this flag fails `validate()`. Loopback never needs it.
    #[serde(default)]
    pub behind_tls_proxy: bool,
    /// Require a verified principal for state-mutating requests
    /// (block/unblock/flush) even when `auth_keys` is empty.
    ///
    /// With `auth_keys` configured, every frame must carry a valid HMAC
    /// envelope and therefore carries a principal, so this setting is not
    /// reachable. It matters only in the no-credentials configuration, where
    /// the role table is skipped entirely and the actor falls back to a
    /// hard-coded `"admin"` — so any local process could `unblock_ip` and
    /// remove an active mitigation.
    ///
    /// Set this to false only to restore the unauthenticated operator path on
    /// a host where that is an accepted risk. A non-loopback bind still
    /// requires `auth_keys` and `behind_tls_proxy`, so this flag can never
    /// widen the exposure past loopback.
    #[serde(default = "default_true")]
    pub require_auth_for_mutations: bool,
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
            max_connections: 1024,
            max_connection_bytes: None,
            read_timeout_ms: None,
            write_timeout_ms: None,
            connection_idle_timeout_ms: None,
            max_line_length: None,
            auth_keys: Vec::new(),
            key_roles: Vec::new(),
            require_auth: false,
            behind_tls_proxy: false,
            require_auth_for_mutations: true,
        }
    }
}
