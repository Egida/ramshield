use super::*;

impl Config {
    /// Validate configuration with sensible bounds and error messages.
    pub fn validate(&self) -> anyhow::Result<()> {
        // Engine config validation
        if self.engine.ram_limit_mb < 64 {
            anyhow::bail!("engine.ram_limit_mb must be at least 64 MB");
        }
        if self.engine.shard_count == 0 || !self.engine.shard_count.is_power_of_two() {
            anyhow::bail!("engine.shard_count must be a power of 2");
        }
        if self.engine.worker_threads > 256 {
            anyhow::bail!(
                "engine.worker_threads must be <= 256 (got {})",
                self.engine.worker_threads
            );
        }

        // XDP mode validation
        match self.xdp.mode.as_str() {
            "skb" | "drv" | "native" => {}
            other => anyhow::bail!("xdp.mode must be 'skb', 'drv', or 'native' (got '{other}')"),
        }

        // Detection config validation
        if self.detection.rps_threshold == 0 {
            anyhow::bail!("detection.rps_threshold must be > 0");
        }
        if self.detection.promote_min_events == 0 {
            anyhow::bail!("detection.promote_min_events must be > 0");
        }
        if !self.detection.relative_factor.is_finite() || self.detection.relative_factor < 1.0 {
            anyhow::bail!("detection.relative_factor must be finite and >= 1.0");
        }
        if !self.detection.relative_floor_rps.is_finite()
            || self.detection.relative_floor_rps <= 0.0
        {
            anyhow::bail!("detection.relative_floor_rps must be finite and > 0");
        }
        if !(1..=u8::MAX as u32).contains(&self.detection.relative_min_samples) {
            anyhow::bail!("detection.relative_min_samples must be in 1..=255");
        }
        if self.detection.relative_min_breaches == 0 {
            anyhow::bail!("detection.relative_min_breaches must be > 0");
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
        if self.ipc.max_connections > 8_192 {
            anyhow::bail!("ipc.max_connections should not exceed 8,192");
        }
        if let Some(mll) = self.ipc.max_line_length
            && mll < 256
        {
            anyhow::bail!("ipc.max_line_length must be >= 256 bytes or None (default 32MB)");
        }

        // Forecasting config validation
        if self.forecasting.enabled {
            if !self.forecasting.ewma_alpha.is_finite()
                || !(0.0..=1.0).contains(&self.forecasting.ewma_alpha)
            {
                anyhow::bail!("forecasting.ewma_alpha must be finite and in range [0.0, 1.0]");
            }
            if !self.forecasting.hw_beta.is_finite()
                || !(0.0..=1.0).contains(&self.forecasting.hw_beta)
            {
                anyhow::bail!("forecasting.hw_beta must be finite and in range [0.0, 1.0]");
            }
            if !self.forecasting.hw_gamma.is_finite()
                || !(0.0..=1.0).contains(&self.forecasting.hw_gamma)
            {
                anyhow::bail!("forecasting.hw_gamma must be finite and in range [0.0, 1.0]");
            }
            if self.forecasting.seasonality_period == 0 {
                anyhow::bail!("forecasting.seasonality_period must be > 0");
            }
            if !self.forecasting.anomaly_zscore.is_finite() || self.forecasting.anomaly_zscore < 1.0
            {
                anyhow::bail!("forecasting.anomaly_zscore must be finite and at least 1.0");
            }
            if !self.forecasting.min_entropy.is_finite() || self.forecasting.min_entropy < 0.0 {
                anyhow::bail!("forecasting.min_entropy must be finite and >= 0");
            }
        }

        // Dashboard config validation
        if self.dashboard.http_addr.is_empty() {
            anyhow::bail!("dashboard.http_addr must not be empty");
        }
        if self.dashboard.max_password_length < 1024 {
            anyhow::bail!("dashboard.max_password_length should be >= 1024 bytes");
        }
        if self.dashboard.max_login_attempts == 0 {
            anyhow::bail!("dashboard.max_login_attempts must be > 0");
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
        if is_public_bind(&self.dashboard.http_addr) && !self.dashboard.tls_enabled {
            anyhow::bail!(
                "dashboard.http_addr binds a public interface ({}) without dashboard.tls_enabled=true — bind 127.0.0.1 or place a TLS-terminating proxy in front and set the assertion",
                self.dashboard.http_addr
            );
        }
        if is_public_bind(&self.ipc.tcp_addr) && self.ipc.auth_keys.is_empty() {
            anyhow::bail!(
                "ipc.tcp_addr binds a public interface ({}) but auth_keys is empty — set HMAC keys or bind 127.0.0.1",
                self.ipc.tcp_addr
            );
        }
        // HMAC authenticates; it does not encrypt. A public bind is only
        // allowed when the operator asserts a TLS proxy in front.
        if is_public_bind(&self.ipc.tcp_addr) && !self.ipc.behind_tls_proxy {
            anyhow::bail!(
                "ipc.tcp_addr binds a public interface ({}) without ipc.behind_tls_proxy — bind 127.0.0.1 or set behind_tls_proxy=true after placing a TLS/mTLS proxy in front",
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
        let mut auth_key_ids = std::collections::HashSet::new();
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
            if !auth_key_ids.insert(id.to_string()) {
                anyhow::bail!("ipc.auth_keys contains duplicate key_id '{id}'");
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
        if let Some(mll) = self.ipc.max_line_length
            && mll < 256
        {
            anyhow::bail!("ipc.max_line_length must be >= 256 bytes or None (default 32MB)");
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
}
