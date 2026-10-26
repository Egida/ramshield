# ramshield-analytics

Bounded-memory streaming primitives: a HyperLogLog, a decaying Count-Min
Sketch and an exponentially weighted Welford estimator.

**Nothing in the daemon uses them yet.** The crate is a workspace member
and the only use in the tree is the `hot_paths` benchmark — no other crate
depends on it, and the detection crate does not call it. The "replaces
unbounded `HashSet<IpAddr>` detection" line is the design intent, not a
shipped fact.

## Data structures

### `SubnetHll` — cardinality estimation per subnet

- Memory: 1024 bytes (`registers: [u8; 1024]`, no heap allocation)
- `insert(&mut self, hash: u64)` and `estimate(&self) -> f64`
- The crate does not hash addresses: the caller supplies a uniform 64-bit
  hash. The crate's own test uses splitmix64, because a weak hash
  collapses the register values and the estimate with it.
- For IPv4 /24 the storage crate's host bitmap is exact in 32 bytes. The
  HLL is for IPv6 /64 subnets, where a bitmap is impossible.
- Small-range correction: below 2.5 × 1024 the estimate switches to
  linear counting.

### `DecayingCountMinSketch` — heavy-hitter frequency

- Memory: 4 rows × 65,536 columns × `AtomicU64` = **2 MiB**, allocated
  as one flat `Vec` on the heap
- `increment(&self, key: u64)`, `estimate(&self, key: u64) -> u64` (the
  minimum across the four rows), `decay(&self)` (halves every counter)
  and `inserted_since_decay(&self) -> u64`
- The decay threshold belongs to the caller: `decay()` halves the
  counters whenever the caller asks. The source suggests calling it once
  a million insertions have accumulated.
- `increment` takes a 64-bit key and no expiry.

### `Welford` — dynamic baseline

- Memory: 24 bytes
- `new(alpha: f32)`, `update(&self, value: f32)`, `mean()`, `variance()`,
  `stddev()`, `count()` and `z_score(value) -> f32`
- `alpha` is the exponential smoothing factor; `Default` uses 0.1.
- `z_score` returns 0.0 while the standard deviation is below 1e-6.
- The update is a compare-exchange on the mean, with the variance stored
  after it.

## Usage

```rust
use ramshield_analytics::cms::DecayingCountMinSketch;
use ramshield_analytics::hll::SubnetHll;
use ramshield_analytics::welford::Welford;

// HLL for IPv6 subnet tracking — the caller supplies the hash
let mut hll = SubnetHll::default();
hll.insert(hash_value);

// Decaying CMS for heavy-hitter tracking
let cms = DecayingCountMinSketch::new();
cms.increment(ip_hash);

// Welford for a dynamic baseline
let welford = Welford::new(alpha);
welford.update(event_value);
```

The types live in their modules; the crate root does not re-export them,
so the import paths are `ramshield_analytics::hll::SubnetHll`,
`ramshield_analytics::cms::DecayingCountMinSketch` and
`ramshield_analytics::welford::Welford`.

## Benchmarks

`benches/analytics_bench.rs` is a compile-only placeholder: three no-op
criterion groups and no measurements. `docs/development/BENCHMARK_BASELINES.md`
carries no analytics entries. The only exercise of this crate is the
workspace `hot_paths` bench, which drives `DecayingCountMinSketch::increment`
and `SubnetHll::insert`.

## Dependencies

Declared: none. The crate is self-contained — `hll.rs`, `cms.rs` and
`welford.rs` use no workspace crate. `criterion` is a dev-dependency.

## Tests

5 unit tests: the HLL's bounded memory and its under-5% error over a
million inserts, the CMS's heap allocation and its decay, and the
Welford's convergence to the mean and its outlier z-score.

## License

MIT
