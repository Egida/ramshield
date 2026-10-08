# RamShield Config Module Readme

## Overview
The `ramshield-config` crate contains the centralized configuration system for the entire RamShield project. It defines the `Config` struct with all subsystem configurations and provides utilities for loading, validating, and applying environment overrides.

## Key Features

### 1. Centralized Configuration
The `Config` struct holds all subsystem configurations in one place:
- Engine: worker threads, RAM limits, sharding settings
- Detection: thresholds, windows, bloom filter sizes, L7 detection rules
- XDP: interface, mode, kernel program toggles
- IPC: TCP addresses, connection limits, timeouts
- Forecasting: HoltWinters parameters, anomaly z-scores
- WAL: write-ahead log directory, durability mode
- Dashboard: HTTP address, session keys, auth hashes

### 2. Fail-Closed by Default
Every field has a conservative fallback. An empty `config.toml` produces a working system that blocks aggressively, logs loudly, and refuses to bind to public interfaces.

### 3. Environment Override System
Environment variables use TOML key notation with double underscores for nested fields:
```bash
RAMSHIELD_ENGINE__RAM_LIMIT_MB=256
RAMSHIELD_DETECTION__RPS_THRESHOLD=50000
RAMSHIELD_L7_RULE__COST_WEIGHT=8.0
```

### 4. Validation Pipeline
1. Load TOML file via serde
2. Apply environment variable overrides
3. Run comprehensive validation checks:
   - RAM limits >= 64 MB
   - Batch windows > 0
   - Batch thresholds > event thresholds
   - Valid HMAC keys (no placeholder values)
   - HTTPS for public-facing dashboards
   - XDP interface name length >= 2

### 5. Hot-Reloading Support
`ConfigHandle` provides atomic configuration updates without service restarts:
```rust
let handle: ConfigHandle = Config::load(".").into_handle();
let cfg = handle.get(); // Access current config
handle.set(new_config); // Atomic swap
```

## Module Structure

### Sections
- `security.rs`: Security-related configurations (Autonomous, Mesh, Native Ingest, Synproxy, Upstream)
- `detection.rs`: Detection configuration including L7 detection rules
- `engine.rs`: Engine configuration
- `forecasting.rs`: Forecasting configuration
- `ipc.rs`: IPC configuration
- `wal.rs`: Write-ahead log configuration
- `dashboard.rs`: Dashboard configuration
- `xdp.rs`: XDP configuration

### Core Files
- `lib.rs`: Main `Config` struct and `from_toml_file()` function
- `env.rs`: Environment variable override application
- `validate.rs`: Comprehensive validation logic
- `net.rs`: Network exposure and binding validation utilities

## Configuration Template

```toml
[engine]
ram_limit_mb = 1024
worker_threads = 4

[detection]
rps_threshold = 10000
window_seconds = 60
batch_window_seconds = 300
batch_threshold_rps = 1000
event_threshold_rps = 500
subnet_burst_ttl_secs = 120
subnet_window_threshold = 500
pre_aggs_max_size = 1000000
emergency_burst_threshold = 500
relative_factor = 5.0
relative_floor_rps = 3.0
relative_min_samples = 8
relative_min_breaches = 3

[l7.rule]
cost_weight = 8.0
baseline_latency_us = 100000
rps_threshold = 500
http2_min_streams = 32
http2_reset_ratio_pct = 80
block_ttl_secs = 300

[xdp]
interface = "eth0"
mode = "drv"
enabled = true
allow_inband_fallback = false

[ipc]
listen_addr = "127.0.0.1:8080"
max_connections = 1024
max_line_length = 262144
auth_keys = ["k1:[REDACTED]"]
key_roles = [{key_id = "k1", role = "Admin"}]
require_auth = true

[forecasting]
enabled = true

[wal]
enabled = true
dir = "/var/lib/ramshield/wal"
compress = true
durability = "hard"
seg_max_bytes = 104857600
retention_max_bytes = 10737418240

[dashboard]
listen_addr = "127.0.0.1:8443"

[mesh]
enabled = false
node_id = "default"
listen_addr = "[::]:9092"
auth_key = "0123456789abcdef"

[upstream]
enabled = false

[autonomous]
enabled = false
syn_pps_per_cpu = 1000
udp_pps_per_cpu = 500
packet_pps_per_cpu = 2000
window_ms = 100

[native_ingest]
enabled = false
interface = "eth0"
max_events_per_sec = 10000

[synproxy]
enabled = false
```

## API Usage

```rust
use ramshield_config::{Config, ConfigHandle};

fn main() -> anyhow::Result<()> {
    // Load configuration from file
    let mut config = Config::load("config.toml")?;
    
    // Apply environment variable overrides
    config.apply_env_overrides()?;
    
    // Validate configuration
    config.validate()?;
    
    // Get handle for hot-reloading
    let handle: ConfigHandle = config.into_handle();
    
    // Access current configuration
    let cfg = handle.get();
    println!("Engine RAM limit: {} MB", cfg.engine.ram_limit_mb);
    
    return Ok(());
}
```

## Module-specific Features

### Security Module (`security.rs`)
- Contains all security-related configurations
- Includes Mesh, Upstream, Autonomous, Native Ingest, and Synproxy configurations
- No duplicate struct definitions

### Detection Module (`detection.rs`)
- Includes L7 detection rules with default values:
  - `cost_weight = 8.0`
  - `baseline_latency_us = 100000`
  - `rps_threshold = 500`
  - `http2_min_streams = 32`
  - `http2_reset_ratio_pct = 80`
  - `block_ttl_secs = 300`
- Promotes default functions to public for use in Config

### Environment Overrides (`env.rs`)
- Typed parsing of environment variables
- Fail-fast validation of invalid environment values
- Secrets never echoed in logs or error messages

### Validation (`validate.rs`)
- Comprehensive validation of all configuration fields
- Fail-closed approach: invalid configs rejected before system startup
- Checks for network exposure risks (public dashboard binds, non-loopback IPC)

## Dependencies
- `serde`: TOML deserialization
- `anyhow`: Error handling and validation
- No internal workspace crate dependencies

## Testing
- 75 fuzz tests in `tests/fuzz.rs`
- Random mutation of config fields to test validation coverage
- Integration tests verify `config.toml` and `config.stress.toml` loading

## Uniqueness

### 1. Fail-Closed Philosophy
Every field has a safe default. The system is designed to block aggressively rather than risk exposure.

### 2. TOML-Style Environment Variables
Consistent naming convention: `RAMSHIELD_SECTION__FIELD=value` for nested structures.

### 3. Modular Architecture
Clear separation of concerns with each subsystem in its own module.

## Integration with Other Crates

- `ramshield-engine`: Reads `config.engine` via `ConfigHandle`
- `ramshield-detection`: Uses `config.detection` and `config.l7`
- `ramshield-xdp`: Accesses `config.xdp` settings
- `ramshield-ipc`: Reads `config.ipc` for server configuration
- `ramshield-forecasting`: Utilizes `config.forecasting` parameters
- `ramshield-wal`: Uses `config.wal` for persistence settings
- `ramshield-dashboard`: Accesses `config.dashboard` for web server config

## Benchmarks
No hot-path benchmarks required — configuration is loaded once at startup and cached. Hot-reload via `ConfigHandle` is O(1) pointer swap.

## Conclusion
The `ramshield-config` crate provides a robust, type-safe configuration system that ensures system safety through fail-closed defaults and comprehensive validation, while maintaining flexibility through environment overrides and hot-reload capabilities.