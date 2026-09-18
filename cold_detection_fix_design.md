# 8s Cold Detection Window — Comprehensive Fix Design

## 1. Root Cause Analysis

### 1.1 EWMA Convergence (Primary: ~3s)
```rust
const ALPHA: f64 = 0.3;  // rate_tracker.rs:1
```
- Time constant τ = 1/α ≈ 3.33 samples
- 95% convergence requires ~3τ ≈ 10 samples
- With batch_window_ms=50, flush_interval=1000ms → ~100ms/sample
- **Result: ~1s for EWMA to rise, ~3s to near-converge**

### 1.2 CUSUM Warmup Guard (Secondary: +600ms)
```rust
pub const CUSUM_WARMUP_SAMPLES: u8 = 6;  // rate_tracker.rs:18
```
- CUSUM only arms after 6 promoted samples
- At 100ms/sample → **600ms delay before drift detection works**

### 1.3 Baseline Initialization Race (Critical: ~2-5s)
```rust
// lib.rs:788-795
let baseline = if rec.baseline_rps == 0.0 {
    rec.baseline_rps = rec.ewma_rps;  // seeds from FIRST ewma (near-zero)
    rec.ewma_rps
} else {
    rec.baseline_rps = ewma_alpha_slow() * rec.ewma_rps + ...;
    rec.baseline_rps
};
```
- **First promotion**: baseline = ewma ≈ 0 (cold)
- Slow EWMA (α=0.033) takes ~30 samples to catch up
- Allowance k = threshold × 0.2 = 200 rps
- During convergence: `inst_rps - (baseline + k)` produces FALSE POSITIVE drift
- CUSUM accumulates phantom evidence → blocks legitimate cold-start traffic

### 1.4 Pulse Tracker Window Alignment (T13: 1/4 bursts blocked)
```rust
// Config default: pulse_window_secs = 6
// T13 pattern: 2s burst, 3s gap → period = 5s
```
- Burst 0 (t=0): opens window, count=1
- Burst 1 (t=5s): within 6s window, count=2 → **FIRES**
- Burst 2 (t=10s): window expired (10-0 > 6), resets → count=1
- **Only 50% of bursts fire** → 1/4 blocking rate (burst 0,2,4... miss)

### 1.5 Bloom Filter Clear Cadence (8s cycle)
```rust
// lib.rs:855
let bloom_clear_ns: u64 = 8 * 1_000_000_000;
```
- Clears every 8s — cold IPs re-inserted, cold-skip path re-evaluates
- Not a root cause but compounds cold-start latency

### 1.6 Promotion Gate (promote_min_events=8)
- New IPs need 8 events in ONE window before full tracking
- First 7 events invisible to EWMA/CUSUM/pulse
- **First detection sample arrives ~window_ms after 8th event**

---

## 2. Implementation Approach

### 2.1 Warm-Start Optimization (Eliminates ~2-5s baseline lag)
**Strategy**: Seed baseline from historical context, not from cold EWMA

```rust
// Add to IpRecord (ramshield-storage/src/lib.rs)
pub struct IpRecord {
    ...
    /// Last known baseline_rps from previous session (WAL replay) or 
    /// from subnet-aggregate heuristic on first promotion
    pub warm_baseline_rps: f64,   // NEW
    pub warm_baseline_source: WarmBaselineSource,  // NEW: enum { None, Wal, SubnetHeuristic, Manual }
    ...
}
```

**Heuristics for warm baseline on first promotion**:
1. **WAL replay**: Restore `baseline_rps` from previous session (already done for blocked IPs, extend to all)
2. **Subnet heuristic**: If subnet has >N promoted IPs, use subnet median baseline_rps
3. **Geo/ASN heuristic**: Pre-seed from known-good ranges (CDN, cloud providers)
4. **Configurable default**: `detection.default_warm_baseline_rps` (default: threshold × 0.1)

### 2.2 Adaptive EWMA Alpha (Reduces convergence from ~3s to ~500ms)
**Strategy**: Start with high α (fast response), decay to steady α

```rust
// rate_tracker.rs - NEW
pub fn ewma_adaptive(prev: f64, sample: f64, sample_count: u64) -> f64 {
    let alpha = if sample_count < 10 {
        0.7  // fast: 95% in ~4 samples (~400ms)
    } else if sample_count < 50 {
        0.4  // medium
    } else {
        ALPHA  // 0.3 steady state
    };
    alpha * sample + (1.0 - alpha) * prev
}
```

### 2.3 Adaptive CUSUM Warmup (Eliminates 600ms guard)
**Strategy**: Dynamic warmup based on traffic pattern

```rust
// rate_tracker.rs - REPLACE constant
pub fn cusum_warmup_samples(threshold: u64, subnet_hot: bool) -> u8 {
    if subnet_hot { 2 } else { 6 }
}
```
- Hot subnets (swarm detected): arm CUSUM in 2 samples (200ms)
- Cold subnets: keep 6 samples (600ms) for stability

### 2.4 Pulse Tracker Window Fix for T13 (Fixes 1/4 bursts blocked)
**Strategy**: Extend window to cover 2 full T13 cycles (2×5s=10s) OR track phase

```rust
// Config addition
#[serde(default = "default_pulse_window_secs")]
pub pulse_window_secs: u64,  // change default from 6 → 12

// OR smarter: track expected next burst time
pub fn pulse_tracker_step_v2(
    prev_count: u8,
    prev_window_start_ns: u64,
    expected_next_burst_ns: u64,  // NEW: phase tracking
    now_ns: u64,
    over_threshold: bool,
    window_secs: u64,
    threshold: u8,
) -> (u8, u64, u64, bool) {  // returns expected_next_burst_ns
    // If over_threshold and we have a period estimate, predict next
}
```
**Simpler fix**: Change default `pulse_window_secs: 6 → 12` (covers 2 full T13 cycles)

### 2.5 Fast-Track Promotion for Returning Traffic
**Strategy**: Bypass promote_min_events for known IPs

```rust
// In flush_batch (lib.rs:573)
let is_returning = bloom_hit || rec.sample_count > 0 || rec.warm_baseline_rps > 0.0;
let effective_promote_min = if is_returning { 1 } else { det.promote_min_events };

if agg.count < effective_promote_min && !subnet_hot && !bloom_hit {
    cold_skipped...
}
```
- Bloom hit = seen recently (within 8s)
- Sample_count > 0 = was promoted before
- Warm baseline = WAL restored

### 2.6 Configurable Cold-Start Thresholds by Traffic Type
**Add to DetectionConfig**:
```rust
pub cold_start_rps_threshold: u64,       // lower threshold for first N samples
pub cold_start_samples: u8,              // how many samples get relaxed threshold
pub warm_start_bypass_promote_min: bool, // skip promote_min for returning
```

---

## 3. File Paths and Changes

### 3.1 Core Changes (Priority Order)

| File | Change | Lines |
|------|--------|-------|
| `crates/ramshield-detection/src/rate_tracker.rs` | Adaptive EWMA alpha, dynamic CUSUM warmup, pulse tracker v2 | 37-199 |
| `crates/ramshield-detection/src/lib.rs` | Warm-start in merge_record, fast-track promotion, baseline seeding | 739-847, 560-585 |
| `crates/ramshield-storage/src/lib.rs` | Add `warm_baseline_rps`, `warm_baseline_source` to IpRecord | ~107 |
| `crates/ramshield-config/src/lib.rs` | New config knobs in DetectionConfig | 43-142 |
| `crates/ramshield-detection/src/batch.rs` | Subnet median baseline heuristic (helper) | NEW function |

### 3.2 Integration Points

| Component | Integration |
|-----------|-------------|
| WAL replay (engine/mod.rs:51-97) | Restore `warm_baseline_rps` for ALL IPs, not just blocked |
| Subnet table (storage) | Add `median_baseline_rps()` query for heuristic |
| Config validation | Bounds for new cold-start params |
| Metrics | Add `cold_start_fast_track_count`, `warm_baseline_seeded_count` |

---

## 4. Integration Plan

### Phase 1: Warm-Start Baseline (Fixes T19 - 83.5% degradation)
**Target**: Eliminate false-positive CUSUM drift during cold start

1. **Add fields to IpRecord** (storage)
   - `warm_baseline_rps: f64`
   - `warm_baseline_source: WarmBaselineSource`
2. **Extend WAL replay** to restore baseline for all IPs
3. **Implement subnet median heuristic** in `merge_record`
4. **Seed baseline from warm_baseline_rps** instead of 0

### Phase 2: Adaptive EWMA/CUSUM (Fixes cold convergence ~3s → ~500ms)
**Target**: Faster convergence without instability

1. **Add `ewma_adaptive()`** with sample-count-based alpha
2. **Replace `CUSUM_WARMUP_SAMPLES` constant** with `cusum_warmup_samples()` function
3. **Thread subnet_hot flag** through to rate_tracker

### Phase 3: Pulse Tracker T13 Fix (Fixes 1/4 bursts blocked)
**Target**: 100% burst detection for 2s-on/3s-off pattern

1. **Change default `pulse_window_secs: 6 → 12`**
2. **Optional**: Add phase-tracking variant for precise alignment

### Phase 4: Fast-Track Promotion (Eliminates promote_min_events delay)
**Target**: Returning traffic tracked from event #1

1. **Add `warm_start_bypass_promote_min` config**
2. **Modify promotion gate** in `flush_batch`
3. **Track metrics** for verification

### Phase 5: Config & Validation
**Target**: Production-ready knobs with safe defaults

1. **Add all new config fields** with defaults
2. **Validation bounds** in Config::validate()
3. **Env var overrides** for all new params
4. **Documentation** in config.toml.example

---

## 5. Verification Strategy

### 5.1 Unit Tests (rate_tracker.rs)

```rust
#[test]
fn ewma_adaptive_converges_fast() {
    // Sample 1-9: alpha=0.7, sample 10-49: alpha=0.4, 50+: alpha=0.3
    // Verify 95% convergence in ≤4 samples for cold start
}

#[test]
fn cusum_warmup_adaptive_subnet_hot() {
    // subnet_hot=true → warmup=2 samples
    // Verify CUSUM arms at sample 2, not 6
}

#[test]
fn pulse_tracker_t13_pattern_fires_all_bursts() {
    // 2s burst, 3s gap, 4 bursts
    // With window=12s, all 4 should fire
}

#[test]
fn warm_baseline_prevents_false_cusum() {
    // Seed baseline=400, steady traffic=400
    // Verify CUSUM stays at 0 (no phantom drift)
}
```

### 5.2 Integration Tests (detection/lib.rs)

```rust
#[test]
fn cold_start_t19_regression() {
    // Simulate T19: new IP, steady 1000 rps from start
    // Before fix: CUSUM fires falsely → block
    // After fix: no block, threat_score < 0.5
}

#[test]
fn returning_ip_fast_track() {
    // IP promoted, then idle >8s (bloom cleared)
    // Returns with 1 event → should promote immediately (bypass promote_min)
}

#[test]
fn warm_baseline_from_wal() {
    // WAL replay with baseline_rps=500
    // First promotion should seed baseline=500, not 0
}
```

### 5.3 Load Test Scenarios

| Scenario | Before Fix | After Fix Target |
|----------|------------|------------------|
| T19: New IP, steady 1000 rps | 83.5% blocked (false +) | 0% blocked |
| T13: 2s burst/3s gap ×4 | 25% blocked (1/4) | 100% blocked |
| Cold start: 500 rps new IP | ~3s to detect | ~500ms to detect |
| Returning IP (bloom hit) | 8 events to promote | 1 event promotes |
| Benign cold ramp (400 rps) | No false CUSUM | No false CUSUM |

### 5.4 Metrics for Production Verification

Add to Metrics:
- `cold_start_fast_track_total` — count of IPs bypassing promote_min
- `warm_baseline_seeded_total` — by source (wal/subnet_heuristic/default)
- `ewma_adaptive_alpha_transitions` — sample_count buckets
- `pulse_tracker_t13_fires` — per-burst fire rate
- `cusum_false_positive_cold_start` — should be 0

### 5.5 Canary Rollout Plan

1. **Shadow mode**: Enable warm-start logging only, no behavior change
2. **Feature flags**: Each phase behind `detection.enable_<phase>` config
3. **Gradual rollout**: 10% → 50% → 100% with metric comparison
4. **Rollback**: Config toggle to revert each phase independently

---

## 6. Risk Assessment

| Risk | Level | Mitigation |
|------|-------|------------|
| Adaptive EWMA overshoot on real attacks | Medium | Cap max alpha at 0.7; CUSUM + pulse as backstops |
| Warm baseline from subnet heuristic polluted by attack | Low | Only use median of IPs with threat_score < 0.3 |
| Pulse window 12s increases false positives | Low | Threshold=2 still requires 2 distinct over-threshold samples |
| Config complexity | Low | Sensible defaults; all behind feature flags |
| WAL replay baseline restore memory | Negligible | One f64 per IP (~8 bytes) |

---

## 7. Execution Checklist

### Phase 1 — Warm-Start Baseline (Week 1)
- [ ] Add `warm_baseline_rps`, `warm_baseline_source` to `IpRecord`
- [ ] Extend WAL replay to restore baseline for all IPs
- [ ] Add `subnet_median_baseline()` to Store
- [ ] Modify `merge_record` to seed from warm baseline
- [ ] Add unit tests for warm baseline seeding

### Phase 2 — Adaptive EWMA/CUSUM (Week 1-2)
- [ ] Implement `ewma_adaptive()` in rate_tracker
- [ ] Replace `CUSUM_WARMUP_SAMPLES` with `cusum_warmup_samples()`
- [ ] Thread `subnet_hot` to rate_tracker calls
- [ ] Add convergence/unit tests

### Phase 3 — Pulse Tracker Fix (Week 2)
- [ ] Change `default_pulse_window_secs()` from 6 → 12
- [ ] Add T13 integration test (4 bursts all fire)
- [ ] Verify no regression on steady attacks

### Phase 4 — Fast-Track Promotion (Week 2)
- [ ] Add `warm_start_bypass_promote_min` config
- [ ] Modify promotion gate in `flush_batch`
- [ ] Add metrics counters
- [ ] Test returning IP with 1 event promotes

### Phase 5 — Config & Polish (Week 2-3)
- [ ] Add all new config fields with validation
- [ ] Add env var overrides
- [ ] Update config.toml.example with documentation
- [ ] Full integration test suite
- [ ] Load test against T19/T13 scenarios
- [ ] Canary deployment plan

---

## 8. Expected Impact

| Metric | Current | Target | Improvement |
|--------|---------|--------|-------------|
| T19 false block rate | 83.5% | <1% | **83x** |
| T13 burst detection | 25% (1/4) | 100% | **4x** |
| Cold start detection latency | ~3-8s | ~500ms | **6-16x** |
| Returning IP promote latency | 8 events / ~800ms | 1 event / ~100ms | **8x** |
| Benign cold ramp false CUSUM | Yes (phantom drift) | No | **Fixed** |

---

## 9. Minimal Diff Summary

**Files Modified** (in priority order):
1. `crates/ramshield-detection/src/rate_tracker.rs` — core algorithmic changes
2. `crates/ramshield-detection/src/lib.rs` — merge_record, flush_batch integration
3. `crates/ramshield-storage/src/lib.rs` — IpRecord schema
4. `crates/ramshield-config/src/lib.rs` — config knobs
5. `crates/ramshield-detection/src/batch.rs` — subnet heuristic helper

**Estimated LoC Change**: ~200 lines (focused, surgical)

**No Breaking Changes**: All new params have safe defaults; existing configs work unchanged.