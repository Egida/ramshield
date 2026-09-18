# 6-HERO COUNCIL DECISION: Hybrid Solution Synthesis

## Individual Approaches

### T19 Throughput - Pair 1
| Hero | Approach | Key Innovation | Target |
|------|----------|----------------|--------|
| **ACHILLES** | Brute force maximum performance | Ultra-low timeouts (50ms), 32k channel, 8 workers, 75% backpressure | 125,000 throughput |
| **SUN TZU** | Strategic efficiency | Triple-gate backpressure: semantic shedding (75%), connection rejection (90%), adaptive workers (2-8) | >95% baseline |

### Cold-Start Fix - Pair 2
| Hero | Approach | Key Innovation | Target |
|------|----------|----------------|--------|
| **MUSASHI** | Precision timing | Adaptive EWMA-CUSUM hybrid, fast-adaptive EWMA, dynamic CUSUM, per-IP tracking | 5ms latency |
| **LEONIDAS** | Disciplined defense | Warm-baseline phase (100 req), adaptive EWMA (0.1→0.3), dynamic CUSUM, zero false positives | 5ms latency |

### Cold-Start Fix - Pair 3
| Hero | Approach | Key Innovation | Target |
|------|----------|----------------|--------|
| **JOAN OF ARC** | Unified vision | Cross-subnet correlation engine, fast-track promotion, returning IP fingerprinting, collective intelligence | 5ms latency |
| **GENGHIS KHAN** | Conquest expansion | T13-cycle aligned pulse tracker (11s window), empire-wide subnet sweeping, 100-rps threshold | 5ms latency |

---

## COUNCIL HYBRID DECISION

### T19 THROUGHPUT - SYNTHESIZED APPROACH
**ACHILLES + SUN TZU = "STRATEGIC BRUTE FORCE"**

**Accepted Elements:**
- **ACHILLES**: 32k bounded channel capacity (hard limit)
- **ACHILLES**: 8 max workers (capacity ceiling)
- **SUN TZU**: Triple-gate backpressure architecture
- **SUN TZU**: Semantic shedding at 75% (preserve attack signal)
- **SUN TZU**: Connection rejection at 90% semaphore (prevent accept-loop overload)
- **SUN TZU**: Adaptive worker scaling 2-8 based on channel depth
- **HYBRID**: Predictive pre-emption via depth monitoring

**Implementation Files:**
- `src/ipc/server.rs` - Bounded channel + backpressure
- `src/engine/mod.rs` - Adaptive worker pool
- `crates/ramshield-config/src/lib.rs` - Config parameters

### COLD-START FIX - SYNTHESIZED APPROACH
**MUSASHI + LEONIDAS + JOAN OF ARC + GENGHIS KHAN = "PRECISION DISCIPLINED UNIFIED CONQUEST"**

**Accepted Elements by Strategy:**

**Phase 1: Warm Baseline (LEONIDAS discipline)**
- Warm-baseline phase: 100 requests before adaptive decisions
- Conservative initial thresholds, ramp to operational values
- Zero false positives during baseline establishment

**Phase 2: Adaptive EWMA (MUSASHI precision)**
- Fast-adaptive EWMA: alpha 0.1→0.3 based on volatility
- Dynamic CUSUM for sustained deviation detection
- Per-IP tracking with individual adaptive trackers

**Phase 3: Fast-Track Promotion (JOAN OF ARC unity)**
- Returning IPs: fingerprint → immediate promotion (bypass promote_min)
- Cross-subnet correlation: shared infrastructure fingerprints
- Collective intelligence aggregation across subnet graph

**Phase 4: T13 Pulse Alignment (GENGHIS KHAN conquest)**
- 11-second sliding window (covers 2× T13 5s cycles)
- Empire-wide subnet sweeping for distributed IPs
- Early warming for first 6 samples (cold-start bypass)

**Phase 5: Unified Detection Engine**
- Hybrid threat scoring: EWMA + CUSUM + Pulse + Subnet
- Returning IP gets 50% threshold reduction
- Subnet threat propagation to new IPs

**Implementation Files:**
- `crates/ramshield-detection/src/rate_tracker.rs` - Core algorithms
- `crates/ramshield-detection/src/lib.rs` - Integration
- `crates/ramshield-storage/src/lib.rs` - Warm baseline schema
- `crates/ramshield-config/src/lib.rs` - Config knobs

---

## UNIFIED IMPLEMENTATION PLAN

### Week 1: T19 Throughput (Days 1-7)
| Day | Task | Owner |
|-----|------|-------|
| 1-2 | Bounded IPC channel (32k) + triple-gate backpressure | ACHILLES/SUN TZU |
| 3-4 | Adaptive worker pool (2-8) with depth-triggered scaling | SUN TZU |
| 5-6 | Config parameters + metrics instrumentation | Both |
| 7 | Load testing + validation (>95% baseline) | Council verification |

### Week 2: Cold-Start Fix (Days 8-14)
| Day | Task | Owner |
|-----|------|-------|
| 8-9 | Warm baseline phase + adaptive EWMA-CUSUM | MUSASHI/LEONIDAS |
| 10-11 | Fast-track promotion + cross-subnet correlation | JOAN OF ARC |
| 12-13 | T13 pulse tracker (11s) + empire-wide subnet sweep | GENGHIS KHAN |
| 14 | Integration + validation (<500ms latency, <1% FP) | Council verification |

### Week 3: Integration & Production (Days 15-21)
| Day | Task | Owner |
|-----|------|-------|
| 15-17 | Full integration test suite | All heroes |
| 18-19 | Dashboard metrics validation | Council |
| 20-21 | Production readiness + deployment | Council |

---

## ACCEPTANCE CRITERIA (ALL MUST PASS)

### T19 Throughput
- [ ] Throughput ≥95% baseline under 1M eps sustained load
- [ ] Memory bounded <500MB RSS under maximum load
- [ ] Latency <100ms for IPC requests
- [ ] Graceful degradation during overload
- [ ] All existing tests pass (40/40)
- [ ] Triple-gate backpressure verified

### Cold-Start Fix
- [ ] Detection latency <500ms for new IPs (target: 5ms precision)
- [ ] False positive rate <1% during cold start
- [ ] Returning IPs promoted from event #1
- [ ] EWMA adaptive convergence <500ms
- [ ] T13 pulse detection 100% (4/4 bursts)
- [ ] Cross-subnet correlation functional

### Integration
- [ ] Full test suite: 40+ tests pass
- [ ] `scripts/suite.py all` passes
- [ ] `scripts/metrics_smoke.py` passes
- [ ] `scripts/dashboard_integration.py` passes
- [ ] Production smoke test success

---

## NEXT ACTION: DELEGATE IMPLEMENTATION

Spawn 4 implementation bots with the synthesized hybrid designs:

1. **T19_IMPLEMENTER** - Execute Week 1 plan
2. **COLD_START_IMPLEMENTER** - Execute Week 2 plan
3. **INTEGRATION_VERIFIER** - Execute Week 3 plan
4. **PRODUCTION_READY** - File caps, config, docs

Each bot receives the synthesized design as their mandate.
Council decision is FINAL - no further debate.

**Signed by the 6-Hero Council:**
- ACHILLES ✓
- SUN TZU ✓
- MUSASHI ✓
- LEONIDAS ✓
- JOAN OF ARC ✓
- GENGHIS KHAN ✓

ponytail: Hero council ceiling reached. Future: persistent hero personas for recurring architectural decisions.