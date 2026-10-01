# Small-Scale Detection Factors

`crates/ramshield-detection/src/small_scale.rs`

## Problem

The engine's primary per-IP gate is an **absolute** rate threshold
(`detection.rps_threshold`, default 1000/s). At small scale — a few
events per second per IP, single host — that gate is a hard floor:
a 30× per-IP flood (50 ev/s vs 1.7 baseline) does not trip it, and
`promote_min_events` (8) skips low-count hosts as "cold" before the
subnet aggregator can see them.

Small-scale detection needs to ask *"is this statistically unusual for
this IP's recent baseline?"* rather than *"is it above a fixed number?"*

## Factors

| # | Factor | Detects | Cost/event | Fires at |
|---|--------|---------|-----------|----------|
| 1 | Poisson log Bayes-factor | sustained rate shift | O(1) | ~2-3 bins of ≥3× rate |
| 2 | Inter-arrival CV (Welford) | bot-regular (CV<0.3) / burst (CV>1.5) | O(1) | after ≥8 samples |
| 3 | Mann-Kendall τ | monotonic slow-ramp | O(n²), n≤16 | 8+ rising bins, τ>0.55 |

Composite: `score = 3.0·LLR̂ + 0.8·CV + 3.0·MK`. Fire when `score ≥ 3.0`.
LLR and MK are each independently sufficient (weight 3.0); CV is a booster.

## Why these three

- **LLR** is the principled small-count test: with only a handful of events
  per window, a fixed threshold either misses the attack or false-positives
  on noise. The Poisson likelihood ratio scales the decision by how much
  evidence the observed count actually carries.
- **CV** separates two attack shapes that look identical to a rate gate:
  a perfectly-regular bot (CV→0) and a bursty flood (CV high) both have the
  *same mean rate* as honest sawtooth traffic.
- **MK** is the only non-parametric factor that catches a **slow ramp** —
  the shape where each individual bin stays under the 3× LLR hypothesis but
  the trend is unmistakable (τ=0.72 on a 2→12 ev/s ramp).

Deliberately **not** included: Rayleigh Z (needs a known pulse period —
the existing `PulseTracker` already covers the T13 5s cycle), Page-Hinkley
(redundant with CUSUM in `rate_tracker.rs`), Welford z-score as a detector
(it is the *foundation* the CV factor uses, not a signal on its own).

## Wiring into the live engine (follow-up)

The module is standalone and fully tested. To feed it live events:

1. Hold a per-IP `SmallScaleTracker` map (DashMap keyed by IP) in the
   engine, size-bounded like `pending_mitigations`.
2. In `batch_processor_loop_from`, after `absorb_or_emergency`, call
   `tracker.record_event(ev.timestamp_ns)`.
3. At each `pre_aggs_flush_interval` boundary, call `tracker.close_bin()`
   for every tracked IP; if `is_alerting()`, emit
   `(ip, BlockReason::HighRps, block_ttl_secs)` through the existing
   `admit_mitigation` gate so the cooldown de-dupes it.
4. Evict trackers on TTL (an IP quiet for > MK_WINDOW seconds with
   `score()==0` is droppable) to bound memory.

This is a **hot-path change** and is intentionally left as a separate
step: it touches the per-event ingest loop, so it needs its own
benchmark + soak before shipping.

## Tests

18 inline tests in `small_scale.rs`, all passing:

- Welford correctness (`welford_basic`, `welford_cv_single_value`)
- MK trend: rising fires, alternating/too-few don't
- LLR: quiet never fires, sustained 3× fires, sub-3× spike doesn't,
  50× single-bin burst does, high-baseline 3× fires
- Scenarios: quiet 2 ev/s never fires; 20 ev/s burst fires; slow ramp
  2→12 fires; bot-regular pattern contributes; burst+CV together fires

Run: `cargo test -p ramshield-detection small_scale`
