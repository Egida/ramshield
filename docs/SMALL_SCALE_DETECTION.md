# Small-Scale Detection

## Problem (Verified Oct 2026)

The engine's per-IP block decision has NO relative/baseline component. Every
path scales to the absolute `rps_threshold` (default 1000/s):

| Path | Condition | 5 ev/s IP w/ 0.17 baseline (30×) |
|------|-----------|----------------------------------|
| `hot` | EWMA > 1000 twice | never |
| `cusum` | S > 1000; allowance k = 0.2×1000 = 200 | drift = (EWMA − 220).max(0) = 0 |
| `pulse` | 2× over-threshold in 6s | never |

CUSUM does NOT eventually catch 5 ev/s — its allowance `k = 0.2 × threshold`
is above 5, so drift never accumulates. **A 30× small-scale anomaly blocks
nothing.**

Also note: `promote_min_events=8` cold-skips such hosts *before* the store
sees them, so even the broken-above path never runs on them.

The correct fix is a **relative** signal: `inst_rps > N × baseline_rps`
(with a baseline floor), applied where `should_block` is computed
(`merge_record`, lib.rs ~1046). It must be tuned for FP against organic
traffic sawtooth (mobile CGNAT, CDN bursts), so it needs its own soak before
it becomes a block decision.

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