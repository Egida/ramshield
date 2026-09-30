/// Benchmarks — sizes and durability modes per the runbook.
///
/// Only run these after semantics are correct; they measure, they do not
/// validate. `cargo bench`.
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use ramwal::Wal;
use ramwal::config::{Config, Durability};

fn bench_dir(tag: &str) -> String {
    let tmp = std::env::var("TMPDIR").unwrap_or("/tmp".to_string());
    let uid = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}/ramwal-bench-{}-{:020}", tmp, tag, uid)
}

fn clean(d: &str) {
    std::fs::remove_dir_all(d).ok();
}

fn cfg(d: &str, dur: Durability) -> Config {
    let mut c = Config::new(d);
    c.durability = dur;
    c.buffer_capacity = 64 * 1024;
    c.seg_max_bytes = 64 * 1024 * 1024;
    c
}

/// Append throughput across payload sizes, buffered.
fn bench_append_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("append_1kb_buffer");
    for size in [1usize, 100, 1024, 4096] {
        let d = bench_dir("sizes");
        let w = Wal::open(cfg(&d, Durability::Buffered)).unwrap();
        let payload = vec![0xABu8; size];
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| {
                w.append(black_box(&payload)).unwrap();
            });
        });
        drop(w);
        clean(&d);
    }
    group.finish();
}

/// Durability modes at a fixed 1 KB payload.
/// `SyncEach` is included for shape only — it fsyncs every append, so
/// expect it to be orders of magnitude slower. That is the contract.
fn bench_durability_modes(c: &mut Criterion) {
    let mut group = c.benchmark_group("durability_1kb");
    let payload = vec![0xCDu8; 1024];

    for (name, dur) in [
        ("Buffered", Durability::Buffered),
        ("Explicit", Durability::Explicit),
        ("SyncEach", Durability::SyncEach),
        ("GroupCommit", Durability::GroupCommit),
    ] {
        let d = bench_dir(name);
        let w = Wal::open(cfg(&d, dur)).unwrap();
        group.bench_function(name, |b| {
            b.iter(|| {
                w.append(black_box(&payload)).unwrap();
            });
        });
        drop(w);
        clean(&d);
    }
    group.finish();
}

/// Recovery throughput: how long `Wal::open` takes over a populated log.
fn bench_recovery(c: &mut Criterion) {
    let mut group = c.benchmark_group("recovery");
    group.sample_size(20);

    for n in [1_000usize, 10_000] {
        let d = bench_dir("rec");
        let w = Wal::open(cfg(&d, Durability::Buffered)).unwrap();
        for i in 0..n {
            w.append(format!("record-{}", i).as_bytes()).unwrap();
        }
        w.sync().unwrap();
        drop(w);

        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let w = Wal::open(cfg(&d, Durability::Buffered)).unwrap();
                black_box(w.recovery_report().records.len());
            });
        });
        clean(&d);
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_append_sizes,
    bench_durability_modes,
    bench_recovery
);
criterion_main!(benches);
