# P7/P8 Adaptive Rate Tracker — Integration Audit

**Status:** source changes committed on `fix/p7-p8-adaptive-rate-tracker`; runtime verification pending.  
**Scope:** integrate the adaptive per-IP CUSUM tracker without removing the existing `rate_tracker` helper API.

## Integration decisions

1. Preserve the existing functions in `crates/ramshield-detection/src/rate_tracker.rs` (`ewma`, `cusum_step_capped`, `cusum_fired`, `ewma_alpha_slow`, `is_exceeded`, and pulse tracking). The detector is appended as additive API; existing callers and benchmark imports are retained.
2. Add `ramshield-forecasting` as a direct dependency of `ramshield-detection`, because the new `TrafficContext` consumes the public `Hypothesis` enum. The dependency graph is acyclic in the current workspace: forecasting depends on config, storage, metrics, and types, not detection.
3. Update `Cargo.lock` to record the new direct dependency.
4. Keep the new detector tests in `rate_tracker::adaptive_tests` so they do not collide with the existing `rate_tracker::tests` module.
5. Use `checked_add` for the sample counter and return `InvalidInternalSampleCount` before state mutation if it overflows.
6. Reset accumulated CUSUM and recovery progress after a timestamp gap greater than `max_gap_tolerance_ns`. A long gap breaks a consecutive-recovery sequence.

## Corrective logic covered

- Timestamp zero is accepted for the first sample; all later timestamps must strictly increase.
- Profiles reject non-finite/invalid thresholds, floors, alpha, clear ratio, and zero warmup/recovery counts.
- Context confidence is validated; stale or future-dated context is not applied.
- Advisory context scaling cannot push `k` or `h` below configured floors.
- Arithmetic results are checked before the commit section; a numerical failure must leave the entire `PerIpRecord` unchanged.
- Breach recovery requires consecutive eligible samples. A burst or excessive timestamp gap resets recovery progress.
- Regression tests cover timestamp replay, invalid profiles, variance overflow, threshold-floor protection, numerical-failure atomicity, gap behavior, and sample-count overflow.

## Operational/integration caveats

- The tracker consumes completed rate samples; it does not enforce the upstream bucket cadence. The aggregator must define and maintain that cadence.
- `TrafficContext::observed_at_ns` and the sample timestamp must use the same monotonic clock domain.
- Adding `recovery_samples` to `DetectorProfile` can require updates to downstream struct literals that do not use `..Default::default()`.
- Review exhaustive matches on `DetectorError` in downstream code.
- Confirm that the detector's `Breached -> Monitoring` transition and CUSUM reset match enforcement expectations before merge.

## Commit sequence

- `build(detection): declare forecasting dependency for adaptive rate tracker`
- `feat(detection): add validated adaptive CUSUM rate tracker`
- `build: update lockfile for detection forecasting dependency`

This audit document is committed separately so the code changes and the verification caveats remain visible in the PR.

## Verification status

**Not run in this environment.** Do not treat this branch as verified or merge-ready until these commands pass in a Rust-enabled checkout:

```sh
cargo fmt --check
cargo test -p ramshield-detection --lib rate_tracker::adaptive_tests
cargo clippy -p ramshield-detection --all-targets --all-features -- -D warnings
cargo test --workspace
```

Also inspect downstream `DetectorProfile` struct literals and exhaustive `DetectorError` matches, then review the final diff and CI results.
