# ramshield-analytics

Bounded-memory analytics primitives used by RamShield. These primitives do not
make enforcement decisions on their own; callers own sampling, lifecycle,
thresholds, and the response to an estimate.

## Components

### `SubnetHll`

HyperLogLog-style cardinality estimate with 1,024 one-byte registers (1 KiB
inside the value). `insert` accepts a precomputed `u64` hash; it does not hash
an IP address itself. `estimate` applies the small-range linear-counting
correction. As with any probabilistic estimator, the result is approximate and
is not an exact membership test. The value is fixed-size; where an IPv4 `/24`
host bitmap is appropriate, storage's 32-byte bitmap is more compact.

### `DecayingCountMinSketch`

Atomic counters are stored in a heap-allocated `Vec<AtomicU64>` with fixed
dimensions. Width determines memory use; the sketch is bounded by those
dimensions, not by the number of observed keys. `increment` updates the counters for
a key and `estimate` returns a collision-prone upper estimate. Explicit
`decay` halves counters; callers decide when to decay. It is a frequency
sketch, not an exact heavy-hitter list or a self-scheduling time window.

### `Welford`

An atomic exponentially weighted mean and variance over `f32` samples, with a
sample count and configured alpha. Despite the type name, this is an EWMA-style
baseline, not the classic exact, unweighted Welford running variance. Its
`z_score` is a statistical signal; callers choose thresholds and policy.

## Operational guidance

- Keep these estimates advisory inputs. Do not treat approximate cardinality
  or sketch counts as exact identities or proof of abuse.
- Choose sketch width from an explicit memory budget and expected collision
  rate. A wider sketch uses more memory and reduces collisions.
- Call `decay` at a cadence appropriate to the caller's traffic window; the
  sketch does not run a timer.
- Configure EWMA alpha and decision thresholds for the target traffic profile.
  Validate with representative traces before enabling automated mitigation.
- Recreate or reset state at explicit lifecycle boundaries. These types do not
  persist their contents across process restarts.

## Dependencies and verification

This crate uses the Rust standard library; it does not depend on `memmap2`.
Run its unit tests with `cargo test -p ramshield-analytics` from the workspace
root.
