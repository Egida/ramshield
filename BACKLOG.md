# RamShield Code Health Backlog

## Build Blockers
- 1 build blocker (E0277 Send violation in ramshield_enforcement::service::mesh::apply_mesh_block, src/engine/boot.rs:382)

## Dead Code (Compiler-reported)
- No dead_code warnings found
- No unused variable warnings found
|- warning: use of deprecated method `std::sync::atomic::Atomic::<u64>::fetch_update`: renamed to `try_update` for consistency
|-     = note: `#[warn(deprecated)]` on by default
|- help: replace the use of the deprecated method

## Suspected Dead Junctions
- ramshield-config:
  - src/lib.rs: pub upstream: UpstreamConfig,
  - src/lib.rs: pub autonomous: AutonomousConfig,
  - src/lib.rs: pub native_ingest: NativeIngestConfig,
  - src/lib.rs: pub synproxy: SynproxyConfig,
  - src/sections/wal.rs: pub allow_volatile_fallback: bool,
  - src/sections/forecasting.rs: pub min_entropy: f64,
  - src/sections/dashboard.rs: pub http_addr: String,
  - src/sections/dashboard.rs: pub admin_password_hash: Option<String>,
  - src/sections/dashboard.rs: pub session_ttl_secs: u64,
  - src/sections/dashboard.rs: pub max_login_attempts: u32,
  - Zero-consumption exports: 49
- ramshield-types:
  - src/events.rs: pub batch_subnet: Option<IpNetwork>,
  - Zero-consumption exports: 1

## Markers (TODO/FIXME/XXX/HACK)
- No TODO/FIXME/XXX/HACK markers found

## Pending Documentation
- Write README.md for: `ramshield-analytics`, `ramshield-cgnat`, `ramshield-mesh`
- Audit existing README.md files against current codebase
- Verify `ramshield-enforcement` README struct fields match actual code
- Update `ramshield-detection` README with L7 rule defaults
