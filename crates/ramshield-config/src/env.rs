use super::*;

impl Config {
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
}
