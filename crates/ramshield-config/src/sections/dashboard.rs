use crate::*;

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
    /// origins). Set true explicitly when behind an HTTPS terminator.
    #[serde(default)]
    pub cookie_secure: Option<bool>,
    /// Max concurrent Argon2 hash operations (default 4). Prevents a flood of
    /// bad logins from saturating the blocking pool with 100ms CPU-bound hashes.
    /// 0 = unlimited (original behavior).
    #[serde(default = "default_argon2_parallelism")]
    pub argon2_parallelism: u32,
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

fn default_argon2_parallelism() -> u32 {
    4
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
            argon2_parallelism: default_argon2_parallelism(),
            trusted_proxies: Vec::new(),
            tls_enabled: false,
            cookie_secure: None,
        }
    }
}
