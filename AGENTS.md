# Repository Guidelines

## Project Structure & Module Organization
The RamShield project is a Rust workspace containing multiple crates:
- `src/` — main binary and library code
- `crates/` — individual library crates (config, detection, enforcement, mesh, storage, types, xdp, ramwal)
- `tests/` — integration tests
- `docs/` — documentation

Core modules:
- **ramshield-xdp** — eBPF XDP engine for network-level DDoS mitigation
- **ramshield-enforcement** — enforcement service that processes XDP decisions
- **ramshield-mesh** — CRDT-based mesh communication for cluster coordination
- **ramshield-storage** — persistent storage layer (WAL, snapshots, recovery)
- **ramshield-types** — shared types (IP networks, timestamps, etc.)
- **ramshield-config** — configuration management
- **ramshield-detection** — detection and mitigation logic
- **ramshield-cgnat** — CGNAT support
- **ramshield-protocol** — protocol definitions

## Build, Test, and Development Commands

### Core Build Commands
```bash
# Compile the workspace
cargo build

# Run all tests
cargo test

# Benchmark suite
cargo bench

# Run specific crate tests
cargo test -p ramshield-xdp
cargo test -p ramshield-enforcement
cargo test -p ramshield-mesh
cargo test -p ramshield-storage
```

### Development Shortcuts
```bash
# Quick local run (requires config.toml)
cargo run --release --features full -- --config config.toml

# Start XDP daemon
./target/release/ramshield-xdp --config config.toml

# Run mesh services
cargo run --release --features mesh
```

## Coding Style & Naming Conventions

### Language Standards
- **Indentation**: 4 spaces (Rust default)
- **Naming**: snake_case for functions/variables, PascalCase for types
- **Modules**: camelCase (e.g., `Aworset`, `BlocklistKey`)
- **Constants**: UPPERCASE_SNAKE_CASE (e.g., `PERMANENT`, `MAX_SYNC_FRAME_ENTRIES`)
- **Documentation**: `///` doc comments for all public APIs

### Tooling
- `cargo fmt` — enforce formatting
- `cargo clippy` — catch potential issues
- `cargo test` — run all tests with full coverage
- `cargo bench` — benchmark hot paths

## Testing Guidelines

### Unit Tests
All crates include comprehensive test suites:
- **XDP**: Idempotent deltas, reorder tolerance, oversized frame rejection, chunking bounds
- **Mesh**: Synchronization limits, chunk preservation, deadline deadlines
- **WAL**: Checkpoint flows, round-trip serialization, post-checkpoint records
- **Storage**: Snapshot integrity, retention pruning, recovery scans

### Integration Tests
- `ramshield-storage/tests/recovery_test.rs` — full recovery cycle
- `ramshield-mesh/tests/mesh_acceptance.rs` — mesh acceptance criteria
- `ramshield-protocol/tests/replay.rs` — protocol message round-trips

### Coverage Requirements
- Every public function must have at least one test
- Critical paths (XDP decision, WAL append, mesh sync) must be exercised
- Edge cases: oversized frames, delayed unblocks, partial syncs, rotation failures

## Security & Configuration Tips

### eBPF/XDP Safety
- All unsafe blocks in `ramshield-xdp` carry `#[allow(unsafe_code)]` with explicit safety comments
- BPF maps use `#[repr(C, align(8))]` for deterministic layout
- Packet bounds are checked before any slice operation (`ptr_at` verifies full packet presence)
- XDP decisions are fail-open (map update failure → continue without blocking)
- Critical: never trust untrusted network data; always validate before use

### Concurrency & Latency
- **Hot path isolation**: XDP runs in dedicated thread with no cross-contamination
- **Mesh CRDTs**: `DashSet` provides lock-free concurrent reads; writers use `Arc<Mutex<Inner>>`
- **WAL layer**: `Arc<Mutex<Inner>>` protects shared state; recovery reopens on poison
- **Segment management**: Separate writer (XDP) from storage (WAL) to prevent interference

### Distributed State Correctness
- **WAL**: Append-only log with checkpoint barriers; recovery scans segments on startup
- **CRDT (Aworset)**: Add/Remove sets with idempotent merges; duplicates are deduplicated
- **Mesh**: Lock-free CRDTs with atomic counters; no central coordinator
- **Retention**: Automatic pruning of closed segments beyond checkpoint LSN

### Memory Management
- **Tombstones**: CRDT removes tombstones on merge; WAL compacts tombstones periodically
- **Leak prevention**: All resources (segments, snapshots) are cleaned on graceful shutdown
- **Buffer pools**: `BufWriter` with fixed-size buffers avoids fragmentation

### Error Handling & Resilience
- **Fail-fast**: XDP panics only on catastrophic map corruption; otherwise continues
- **Poisoning**: WAL transitions to `Poisoned` state on write failure; caller must reopen
- **Deadlines**: Timeouts on all remote operations; graceful degradation
- **Circuit breakers**: Mesh rejects sync requests when bandwidth exceeds thresholds

## Performance & Scaling

### Throughput Targets
- **XDP**: ~1M packets/sec per core (kernel bypass)
- **Mesh**: 10K ops/sec per node (CRDT merge is O(log n))
- **WAL**: 50K appends/sec (batch flushing, LZ4 compression)

### Resource Limits
- Max segment size: 64 MB (configurable)
- Max WAL entries: 4096 (prevents unbounded growth)
- Max concurrent writers: 64 (semaphore-controlled)
- Max in-flight sync batches: 256 (prevents memory spikes)

## Operational Checklist

### Pre-Deployment
- [ ] Run full test suite: `cargo test`
- [ ] Verify WAL recovery: `ramshield-storage::replay`
- [ ] Confirm mesh connectivity: `meshes::ping_all()`
- [ ] Check eBPF program loaded: `bpftool prog show ramshield-xdp`
- [ ] Validate CRDT convergence: `mesh::convergence_test()`
- [ ] Review logs for poison events: `ramshield-enforcement::log`

### Monitoring
- **Metrics**: LSN progression, segment lag, WAL flush latency
- **Alerts**: Poisoned WAL, checkpoint delays > 30s, memory growth > 10GB
- **Health checks**: Endpoint liveness, XDP throughput, storage I/O

### Rollback
- Versioned binaries in `target/`
- Config backup before major changes
- Blue-green deployment for XDP (separate build artifact)
