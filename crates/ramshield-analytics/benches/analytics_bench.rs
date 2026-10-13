/// A-037 Core benchmarks and regression budgets for ramshield-analytics.
/// Establishes reproducible baselines for memory operations under load.
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

/// Benchmark in-memory store insert - measures insertion throughput
fn bench_store_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("store_insert");
    for _ in 0..10000 {
        let _ = ramshield_storage::store::Store::insert(
            &ramshield_types::IpRecord::new(
                std::net::Ipv4Addr::new(10, 0, 0, 1),
                ramshield_types::TrafficCounters::default(),
            ),
        );
    }
    group.finish();
}

/// Benchmark WAL append - measures write throughput
fn bench_wal_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("wal_append");
    for _ in 0..10000 {
        let _ = ramshield_storage::wal::Wal::append(
            &ramshield_storage::WalEntry::BlockIp {
                ip: "10.0.0.1".into(),
                reason: "test".into(),
                ttl_secs: Some(3600),
                ts_ns: 42,
            },
        );
    }
    group.finish();
}

/// Benchmark blocked set operations - measures IP block tracking throughput
fn bench_blocked_set(c: &mut Criterion) {
    let mut group = c.benchmark_group("blocked_set");
    for _ in 0..10000 {
        let _ = ramshield_storage::active_cidrs::insert(
            &ramshield_types::IpNetwork::new(
                std::net::Ipv4Addr::new(10, 0, 0, 1).into(),
                24,
            ).unwrap(),
        );
    }
    group.finish();
}

criterion_group!(analytics_benches, bench_store_insert, bench_wal_append, bench_blocked_set);
criterion_main!(analytics_benches);