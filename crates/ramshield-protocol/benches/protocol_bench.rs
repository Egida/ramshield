/// Establishes reproducible baselines for protocol operations under load.
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
/// A-037 Core benchmarks and regression budgets for ramshield-protocol.
use ramshield_storage::{Store, WalEntry};
use ramshield_types::IpNetwork;
use std::hint::black_box;

/// Benchmark IPC message serialization - measures message encoding throughput
fn bench_ipc_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("ipc_serialize");
    for _ in 0..10000 {
        let _ = ramshield_protocol::ipc::serialize_message(&ramshield_types::IpcMessage::new(
            ramshield_types::IpcCommand::BlockIp {
                ip: "10.0.0.1".into(),
                reason: "test".into(),
                ttl_secs: Some(3600),
            },
        ));
    }
    group.finish();
}

/// Benchmark HMAC authentication - measures signature verification throughput
fn bench_hmac_verify(c: &mut Criterion) {
    let mut group = c.benchmark_group("hmac_verify");
    for _ in 0..10000 {
        let _ = ramshield_protocol::auth::verify_hmac(
            b"test-key-16-bytes",
            b"test-message",
            b"expected-signature",
        );
    }
    group.finish();
}

/// Benchmark replay detection - measures replay window check throughput
fn bench_replay_check(c: &mut Criterion) {
    let mut group = c.benchmark_group("replay_check");
    for _ in 0..10000 {
        let _ =
            ramshield_protocol::replay::check_replay(&ramshield_types::ReplayWindow::new(1000), 42);
    }
    group.finish();
}

criterion_group!(
    protocol_benches,
    bench_ipc_serialize,
    bench_hmac_verify,
    bench_replay_check
);
criterion_main!(protocol_benches);
