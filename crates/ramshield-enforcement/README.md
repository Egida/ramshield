# ramshield-enforcement

```text
EnforceCommand (from detection/forecasting)
           │
           ↓
    EnforcementService actor
           │
           ↓
    WAL append (optional)
           ↓
    Store mutation + TTL scheduling
           ↓
    XDP projection
```

## Why it exists

Detection and forecasting produce decisions — "block this IP for 3600 seconds" or "unblock it now." But a decision sitting in memory is worthless. This crate executes those decisions: writes them to the WAL (crash safety), applies them to the Store (in-memory state), pushes them to XDP (kernel-level packet drop), and schedules automatic unblocking after the TTL expires. Without enforcement, detection is just expensive logging.

## How it works

### EnforcementService

An actor that owns enforcement state and serializes enforcement decisions. The
runtime supplies the command channel; its capacity is a runtime configuration
detail and should not be treated as a lossless buffer or as proof that detection
cannot experience backpressure.

```rust
pub struct EnforcementService {
    store: Arc<Store>,
    metrics: Arc<Metrics>,
    xdp: Box<dyn XdpApplier>,
    wal: Option<Arc<Wal>>,
    processed_results: HashMap<Uuid, EnforceResult>,  // dedup by decision_id
    processed_order: VecDeque<Uuid>,        // LRU eviction for dedup set
    blocked_ips: HashSet<IpAddr>,           // current block state
    expirations: HashMap<IpAddr, (u64, usize)>,  // TTL expiry timestamps + bucket position
    cidr_expirations: HashMap<IpNetwork, Instant>,
    buckets: BTreeMap<u64, Vec<IpAddr>>, // time-bucketed expiration (second buckets)
    epoch: Instant,
    shutdown: Arc<AtomicBool>,
    drops_by_blocked: HashMap<IpAddr, u64>,  // per-IP XDP drop counters
    mesh_blocklist: Option<Arc<ramshield_mesh::aworset::AworsetBlocklist>>,
    mesh_handle: Option<ramshield_mesh::MeshHandle>,
    mesh_applied_ips: HashSet<IpAddr>,
    mesh_operator_suppressions: HashSet<IpAddr>,
    checkpoint_shared: Option<Arc<CheckpointShared>>,
}
```

### Decision Processing

Every incoming command is checked against `processed_results` — a `HashMap<Uuid, EnforceResult>` that deduplicates by `decision_id`. This prevents the same block decision from being applied twice if detection fires multiple times in rapid succession (the `processed_order` `VecDeque` evicts the oldest entries when the set exceeds 65K).

Processing path:

1. **Block command**: after validation and any configured checkpoint barrier,
   the WAL append occurs first when WAL is configured. Only then is userspace
   state mutated and TTL scheduled. XDP is attempted as a projection after the
   commit. A failed projection is reported as not applied and marks projection
   health stale; it does not undo the durable userspace decision. Whether the
   daemon starts or continues without XDP depends on its XDP fallback settings.

2. **Unblock command**: the configured checkpoint barrier and WAL append happen
   before userspace state changes. Expiration state is removed and the XDP
   projection is attempted afterward. A projection failure is observable and
   requires reconciliation; it is not evidence that the authoritative state
   was rolled back.

### TTL Ring

Block decisions include a `ttl_secs` field. When a block is applied, `schedule_expiration(ip, instant)` places the IP into a second-bucketed `BTreeMap<u64, Vec<IpAddr>>`. The bucket second is derived from the expiry Unix-ns timestamp, bucketed by whole second. This makes re-block (TTL refresh) O(1): detach via swap-remove from the old bucket, attach to the new. A background loop wakes every second, checks the earliest bucket, and issues unblock commands for any IPs whose TTL has elapsed.

IP expiry uses monotonic `Instant` — not `SystemTime` — so wall-clock
adjustments do not change its deadline. The bucketed index tracks bucket and
position to avoid scanning all pending IP expirations on every tick. Expiry
resolution is bucketed at one-second granularity. CIDR expirations are tracked
separately.

### WAL Replay

On startup, `replay_wal_into_store()` reads the WAL and replays all block entries into the Store. It returns pairs of `(IpAddr, remaining_secs)` — IPs that were blocked at crash time with their remaining TTL. `restore_expirations()` reschedules these in the TTL ring, so blocking survives daemon restarts.

### XdpApplier Trait

The kernel XDP interface is abstracted behind a trait — enabling testing without root privileges:

```rust
pub trait XdpApplier: Send + Sync {
    fn apply_block(&self, ip: IpAddr) -> Result<(), EnforcementError>;
    fn apply_unblock(&self, ip: IpAddr) -> Result<(), EnforcementError>;
    fn reconcile(&self, blocked: &HashSet<IpAddr>) -> Result<(), EnforcementError>;
}
```

Two implementations exist:
- `StubXdpApplier`: no-op, used in tests and when XDP is disabled.
- `AyaXdpApplier`: loads the compiled BPF program from `ramshield-xdp::BPF_ELF`, attaches to a network interface via `aya`, and maintains a hash map of blocked IPs that the kernel program checks on every packet.

### Block Reasons

```rust
pub enum BlockReason {
    HighRps,
    SubnetBatch,
    ForecastAnomaly,
    EntropyAnomaly,
    ManualBlock,
    L7HighRps,
    L7Cost,
    Http2StreamAbuse,
}
```

Each reason maps to a stable string (`"high_rps"`, `"subnet_burst"`, `"forecast_anomaly"`, `"entropy_anomaly"`, `"manual"`, `"l7_high_rps"`, `"l7_cost"`, `"http2_stream_abuse"`) for wire protocol and Prometheus labels via the `as_str()` method. `from_reason_str()` provides reverse mapping.

## Uniqueness

**Deduplication prevents block storms.** During a DDoS, detection may fire the same block decision hundreds of times per second. The `processed_results` HashMap ensures each `decision_id` is applied exactly once — preventing redundant XDP map writes and WAL appends that would consume disk and CPU.

**Time-bucketed IP expiry.** IP expirations are grouped by deadline second in
a `BTreeMap`; due buckets can be drained without scanning every pending IP.
CIDR expiry uses its own index.

**Projection separation.** The userspace decision remains authoritative if an
XDP update fails, and projection health exposes the failure. Startup fallback
is controlled by configuration: deployments that require kernel enforcement
must verify XDP is active and must not assume an in-band decision drops packets
in the kernel.

## Dependencies

**Reads from:** `ramshield-types` (`EnforceCommand`, `EnforceAction`, `BlockReason`, `IpNetwork`), `ramshield-storage` (`Store`, `Wal`), `ramshield-metrics` (`Metrics`), `ramshield-config` (for WAL config), `ramshield-mesh` (CRDT blocklist).

**Written by:** `ramshield-detection` (sends `EnforceCommand` via channel), `ramshield-forecasting` (sends `EnforceCommand` via channel).

**Read by:** `ramshield-dashboard` (block log), `ramshield-metrics` (block counters).

## Benchmarks

No standalone benchmarks — enforcement latency is measured end-to-end in `benches/hot_paths.rs` through the Store update path. XDP map operations are measured separately in kernel-space benchmarks (not yet implemented).

## Testing

The crate tests cover decision idempotency, TTL expiry, WAL replay, XDP
projection success and failure, concurrent block/unblock, mesh projection, and
checkpoint coordination. Test count changes as cases are added; use Cargo's
current test output instead of relying on a count in this document.
