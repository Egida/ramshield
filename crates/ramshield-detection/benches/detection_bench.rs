/// A-037 Core benchmarks and regression budgets for ramshield-detection.
/// Establishes reproducible baselines for detection operations under load.
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use ramshield_detection::rate_tracker::PulseTracker;

/// Benchmark rate tracker pulse observation - measures throughput
fn bench_rate_tracker_pulse(c: &mut Criterion) {
    let group = criterion::BenchmarkGroup::new(
        c,
        criterion::Axis::Threads,
        criterion::Style::Timer,
    );
    for _ in 0..10000 {
        let tracker = PulseTracker::new();
        let _ = tracker.observe(std::time::Instant::now().elapsed().as_nanos() as u64, 1);
    }
    group.finish();
}

/// Benchmark rate tracker can_emit - measures pulse decision throughput
fn bench_rate_tracker_can_emit(c: &mut Criterion) {
    let group = criterion::BenchmarkGroup::new(
        c,
        criterion::Axis::Threads,
        criterion::Style::Timer,
    );
    for _ in 0..10000 {
        let tracker = PulseTracker::new();
        let _ = tracker.can_emit_pulse(std::time::Instant::now().elapsed().as_nanos() as u64);
    }
    group.finish();
}

criterion_group!(detection_benches, bench_rate_tracker_pulse, bench_rate_tracker_can_emit);
criterion_main!(detection_benches);