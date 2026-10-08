# Small-Scale Detection

## Problem (Verified Oct 2026)

The original 0.3.x per-IP block decision had NO relative/baseline component.
The closure path below adds an **experimental, opt-in** relative detector; the
default product behavior remains absolute/CUSUM/pulse based:

| Path | Condition | 5 ev/s IP w/ 0.17 baseline (30×) |
|------|-----------|----------------------------------|
| `hot` | EWMA > 1000 twice | never |
| `cusum` | S > 1000; allowance k = 0.2×1000 = 200 | drift = (EWMA − 220).max(0) = 0 |
| `pulse` | 2× over-threshold in 6s | never |

CUSUM does NOT catch a 5 ev/s anomaly against the 1000/s absolute policy —
its allowance `k = 0.2 × threshold` is above 5, so drift never accumulates.
That historical gap is why the relative path exists, but it remains opt-in.

Also note: `promote_min_events=8` cold-skips such hosts *before* the store
sees them, so even the broken-above path never runs on them.

The correct fix is a **relative** signal: `inst_rps > N × baseline_rps`
(with a baseline floor), applied where `should_block` is computed
(`merge_record`, `crates/ramshield-detection/src/merge.rs`). It must be tuned for FP against organic
traffic sawtooth (mobile CGNAT, CDN bursts), so it needs its own soak before
it becomes a block decision.

## Empirical confirmation (Oct 2026)

A probe drove the real `DetectionEngine` (observation, not theory):

**Barrier 1 — cold-skip gate.** Default config, `promote_min_events=8`.
5 ev/s IP flushed 20× (100 events total). Result: `promoted_ips: 0`,
IP never stored, 0 blocks. Every flush of ≤5 events is cold-skipped before
`merge_record` runs — EWMA/CUSUM never even instantiate.

**Barrier 2 — absolute threshold.** Forced `promote_min_events=2`,
kept `rps_threshold=1000`. The IP promoted and accumulated 21 samples:

```
ewma_rps:     6.25
baseline_rps: 2.84
cusum_s:      0.00      <- 21 samples of positive drift, all 0
threat_score: 0.004
block_state:  Clean
blocks_emitted: 0
```

`cusum_allowance(1000)` = 200. Every sample's drift `(inst − baseline − 200)`
is negative, so `cusum_s` is permanently 0 — it never starts. The CUSUM design
test (`rate_tracker.rs:91`) confirms intent: it fires only for "600 rps from
50 baseline," i.e. drift past allowance. A 5 ev/s anomaly is invisible to
every block path.

So two independent mechanisms each do their job:
- **cold-skip** bounds memory under cardinality-swarm (the failure the
  per-IP-tracker approach would reintroduce);
- **absolute threshold + CUSUM allowance** bounds FP under organic sawtooth.

Closing the gap means adding a *third* path — a relative signal bounded in
ways both are not — not weakening either.

## Status

`small_scale.rs` was written (Poisson-Gamma LLR, inter-arrival CV,
Mann-Kendall τ) with 18 passing tests but was **removed** — it was dead code
(registered in lib.rs, zero production callers) and its proposed live wiring
(a per-IP DashMap of trackers in the flush path) was the wrong shape: it
re-introduces attacker-cardinality state the sketch architecture exists to
avoid, and adding a block-emitting side-map to the cold-skip branch is a
hot-path change that needs a benchmark + soak before it can ship.

The factor formulas are archived as reference for the relative-threshold
work when it is properly scoped:

- **Poisson log Bayes-factor** — sustained rate shift at ≥3× baseline within
  ~2 bins. Principled small-count test.
- **Inter-arrival CV (Welford)** — bot-regular (CV<0.3) vs burst (CV>1.5),
  same-mean shapes a rate gate can't separate.
- **Mann-Kendall τ** — non-parametric slow-ramp detector (τ=0.72 on a
  2→12 ev/s ramp), the shape LLR structurally misses (each bin < 3×).

Composite (reference): `score = 3.0·LLR̂ + 0.8·CV + 3.0·MK`, fire ≥ 3.0.
LLR and MK each independently sufficient; CV is a booster.

## Experimental relative closure path

The opt-in relative detector is implemented on promoted records without a
per-IP side map. It compares each sample against the prior slow baseline,
requires a mature sample count, and requires a consecutive breach streak.
It is disabled by default and is not a 1.0 default detection guarantee.
